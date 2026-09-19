# E2E design: compose existing Operations and States

Historical design proposal. The current contract is [RFC_REFACTOR_DSL.md](../RFC_REFACTOR_DSL.md);
its native Pending pacing supersedes the Runtime deadline proposed here. Production acceptance
and the exact replacement mapping are recorded in [dsl-phase-b.md](dsl-phase-b.md).


The [DSL refactoring RFC](../RFC_REFACTOR_DSL.md) supersedes this document's authoring and execution
API sketches. It specifies typed State/Operation construction, capability-owned typed injection,
compilation to Program, automatic executable association, and original failures without mandatory
root maps. In particular, `author_operation`, `config.contracts()`, the mutable construction DSL,
and direct execution of a selection/Operation below are superseded proposals. The product story,
independent assertions, infrastructure, and required recovery coverage remain requirements.

This is the target design for public usage 2 in the
[public interfaces RFC](../RFC_RESHAPING_PUBLIC_FACING_N_TESTS.md). It specifies the experience to
implement, not a claim that the proposed APIs already exist. Current API limitations must not
become acceptance criteria. This document changes no executable test or managed task.

## Objective and user story

A Rust caller reuses the documented components of the maintained `ContractDeploymentLifecycle`
Operation, which deploys, configures, observes, validates, and reports on a contract. The caller
constructs a new operation through the existing `OperationExpansion` DSL, inserting one existing
production Pure State. No original lifecycle instance is needed merely to access those components. The test
expresses recomposition, not implementation of an operation's missing internals.

The standard operation and recomposed workflow have visibly different results:

```text
configuration: initial value 42, increment 42

maintained operation:
  Deploy -> Configure -> Observe -> Validate -> Report => 42
recomposed workflow:
  Deploy -> CheckedAddConfigurationValue -> Configure -> Observe -> Validate -> Report => 84
```

The addition executes as a real Pure State between deployment and configuration. It consumes the
configured value, not an earlier on-chain observation. There is one deployment and one configuration
transaction per run. Configure uses the created address; Observe uses the configuration transaction's
target and receipt anchor. A later case may configure/observe twice, but that is not this baseline.

The whole operation must exist as maintained production functionality before this E2E consumes it.
Deploy and Configure are meaningful Effect States; Observe is a Read State; Validate and Report
are Pure States. These are the operation's public reusable components, not independently handwritten
fixture equivalents or one-State Operation wrappers. Production also owns the checked-add State and its typed argument-slot binding.
The test introduces no struct, type alias, State implementation, failure map, codec, or registration
list. Those obligations belong to production for these already-supported components.

The existing transaction and anchored Read implementations provide the underlying semantics. Their
production package owns the reusable workflow/context; Runtime remains generic and acquires no
built-in EVM workflow knowledge. This remains a library-facing test, independent of Application or CLI.

## Operations, States, and typed step selections

An Operation is a reusable recipe that groups meaningful States and, where useful, child Operations.
A State owns one deterministic execution contract. A Program is the final expanded State sequence
that Runtime executes. Selecting a component adds an occurrence; it does not define a new State or
Operation implementation.

A typed step selection combines an existing State type with its required authoring setup. Calling
`states.deploy()` describes an Effect occurrence with its capability and public binding; it performs
no deployment or other IO. `body.effect(states.deploy())` includes that occurrence and its injection
in the authored sequence. Use `states` for the local collection and `operation` for the authored
composition. The accessors return typed selections, not running State instances; document that
behavior without introducing a separate everyday name. Concrete Operation types retain descriptive
names such as `ContractDeploymentLifecycle`.

| Component | Mode | Complete caller-facing role |
| --- | --- | --- |
| Deploy | Effect | Deployment request to deployed-contract context, including required transaction support. |
| CheckedAddConfigurationValue | Pure | Checked addition on the numeric configuration argument, preserving deployment facts. |
| Configure | Effect | Deployed-contract context and effective argument to configured-contract facts. |
| Observe | Read | Configured-contract facts to observation at that call's target and receipt anchor. |
| Validate | Pure | Check the observed getter equals the effective configured argument; return checked agreement or a typed mismatch. |
| Report | Pure | Construct the declared terminal report from validated facts, retaining public inputs, effective value, transaction facts and observation evidence. |

Validate checks the declared lifecycle relationship, not chain/transaction integrity already proved
by lower owners. Its expected value comes from the effective command input, not a hard-coded `84`.
Report owns useful public result projection rather than an identity step added solely to match a
name. The test independently asserts the expected values and evidence.

The production package implements the selection accessors once. This sketch shows the intended
shape, not existing or complete signatures; ordinary root maps and occurrence policy are omitted:

```rust
impl ContractLifecycleStates {
    pub fn deploy(&self) -> EffectSelection<Deploy, EvmTransactionEffect> {
        EffectSelection::new(self.binding.clone())
    }

    pub fn configure(&self) -> EffectSelection<Configure, EvmTransactionEffect> {
        EffectSelection::new(self.binding.clone())
    }

    pub fn observe(&self) -> ReadSelection<Observe, EvmAnchoredContractCallRead> {
        ReadSelection::new(self.binding.route.clone())
    }

    pub fn validate(&self) -> PureSelection<ValidateConfiguration> {
        PureSelection::new()
    }

    pub fn report(&self) -> PureSelection<BuildContractReport> {
        PureSelection::new()
    }
}
```

The mode-specific selection types belong to the framework authoring layer; domain packages construct
them without depending on Runtime. The concrete State types, intermediate contracts, and ordinary
maps are production-owned. These selections carry type information and immutable public setup, not secret keys or provider
handles. Runtime binds their capability identities to the separately supplied live handles.
Selecting the addition step similarly binds a maintained checked arithmetic State to a documented
typed numeric slot. Neither selection construction nor the authoring closure executes State logic.

## Injection and standalone States

Injection already belongs to `CapabilityInjection<S>`. The current
`OperationExpansion::effect` and `read` (at revision `660285d0`) directly expand
an exact capability/State pair through its prefix, designated State, and successful suffix. There
is no requirement for each selected Effect State inside a composition to have a child Operation
wrapper. For a transaction selection the expansion includes:

```text
ReserveNonce -> PrepareTransaction -> designated Deploy/Configure Effect -> ProjectOutcome
```

Each expanded State retains its own execution, persistence, and recovery boundaries. The suffix
runs on designated success; it is not a finally handler. Injection neither makes the whole sequence
atomic nor moves IO into deterministic State code.

Today's root `expand_program` entry accepts only an Operation. The existing `EvmTransaction`
Operation validates its root input and delegates to one `body.effect(...)` call. That root-entry
restriction explains standalone-use friction; injection itself does not require the wrapper.

The target generalizes the same authoring entry to accept an Operation or a checked Pure/Read/Effect
selection. Do not add a competing `State::expand` hook or require every State author to implement
expansion. Capability-owned injection remains the single owner. Standalone use should be possible:

```rust
runtime.execute(run_id, states.deploy(), checked_deployment_input).await?;
```

This runs the same injection/lowering as nested selection without a caller-written one-State
Operation. Removing a wrapper must preserve its root checks, including binding agreement and
transaction action-mode validation, in the domain descriptor or checked occurrence qualification.
Those checks must still happen before admission/provider entry where their inputs are available.

Expose the selection's `ExpandedInput`, `ExpandedOutput`, and `ExpandedFailure` to its caller.
The underlying designated State can accept prepared facts and return executed facts, while the
public Deploy selection accepts a deployment request and returns completed deployment facts.
Every raw State ABI remains exactly associated internally; hiding intermediates must not weaken
their validation or require callers to manufacture prepared transaction data.

## Configuration supplies the input

Load a checked production configuration before constructing the workflow. The fixture supplies the
configuration document and compiled contract artifact; the maintained configuration boundary owns
parsing and validation. The document selects the artifact, public binding, setter/getter contracts,
initial argument, increment, checked gas/fee options, and admitted recovery policy. Secret signer
material and provider/Store handles are supplied separately as explicit capabilities.

Its relevant value fragment is illustrative, not a complete or implemented config schema:

```json
{
  "initial_value": "42",
  "increment": "42",
  "configure_function": "configure(uint256)",
  "observe_function": "value()"
}
```

Production owns `ContractWorkflowConfig`, its initial/intermediate context contracts, and checked
result access. `config.initial_input()` is a production conversion, not test-owned construction
of `ContractContext` or transaction recipe aliases. Source code chooses the sequence; configuration
supplies its inputs and supported options. No arbitrary configured program or JSON-path wiring is
needed for this scenario.

The initial value must remain a checked number until Configure constructs its calldata and command.
The Pure State produces a new context with `84`, preserving deployment facts and unrelated fields.
The admitted initial input still records `42`; it is never rewritten. Command construction occurs
before the configuration Effect is prepared and acknowledged. Reconciliation thereafter uses the
retained command and fees, never reruns addition to modify a pending command.

## One constructor using the current authoring DSL

The maintained lifecycle and caller-defined compositions use one ordinary authoring constructor
with the current `OperationExpansion::operation`, `effect`, `pure`, and `read` mechanisms. Reuse
comes from the production components, not a transformation method on an existing operation.

`author_operation` below is a proposed name for that common constructor, not an existing function
or a test helper. The configuration supplies the production root contracts and validation through
the illustrative `config.contracts()` accessor. Exact generic, root-map, and occurrence arguments
are abbreviated: production typed step selections must supply them, with inference where possible.
The signatures remain to be validated; they introduce no parallel DSL or lowering path.

```rust
let config = ContractWorkflowConfig::load(config_path).await?;
let states = ContractLifecycleStates::new(&config)?;

let operation = author_operation(config.contracts(), |body| {
    body.effect(states.deploy())?;
    body.pure(states.add_to_configuration(config.increment()))?;
    body.effect(states.configure())?;
    body.read(states.observe())?;
    body.pure(states.validate())?;
    body.pure(states.report())
})?;

let runtime = Runtime::builder(store)
    .capabilities(capabilities)
    .execution_policy(attempt_policy)
    .build()?;

let outcome = runtime
    .execute(run_id, operation, config.initial_input())
    .await?;

assert_eq!(expect_success(outcome).value(), U256::from(84));
```

The closure only declares the sequence through the existing DSL. It does not implement addition,
ABI encoding, provider IO, or progression. `add_to_configuration` selects a maintained checked-add
Pure State for a documented typed configuration-argument slot; it must not be a hidden test helper
or a closure that executes arithmetic during authoring.

The maintained lifecycle's constructor uses the same authoring constructor and step selections,
omitting only the added arithmetic State. Its default sequence produces `42`. The test uses the
same components to declare a new sequence producing `84`; it does not copy their implementations,
inspect private lowered positions, or edit an admitted Program.

Both construction paths qualify the declared root input/output/failure contracts, root validation,
and the complete sequence. A missing deployment, incompatible input, wrong binding, or unsupported
failure map must be rejected. Reusing valid components does not make an arbitrary sequence valid.
There is no separate operation-transformation constructor or named-step replacement machinery.
A constructor for the maintained product may select its default recipe, but it delegates to the
same authoring path rather than owning another validation/lowering implementation.

## Production ownership and executable association

The production workflow exposes checked configuration, context contracts, documented components,
and result access. Callers do not need to name every generic intermediate type merely to reuse it.
Context representation stays typed: a named numeric argument slot is not a global context bag.
Existing fact constructors own transaction/evidence relationships and sibling preservation.

Retain the single `OperationExpansion` lowering path. Selecting a component must supply its exact
State, capability, map, handler, and codec requirements to Runtime association, including injected
transaction stages. A Program-owned implementation sink remains a candidate mechanism, subject to
a consuming proof; its purpose is to remove duplicate registration, not add a second inventory.
Domain crates depend inward on Program, never Runtime, Store, or live handles. Authoring callbacks
are not retained for execution.

Explicit capabilities remain distinct from executable selection because they supply external
authority. Runtime preparation qualifies the initial input, replacement sequence, exact implementation
requirements, and declared bindings before admission/provider entry. It performs no external
workflow execution. Observation-derived inputs and changing live facts still require their existing
Runtime/adapter checks. Failed preparation must not expose a usable partial assembly.

The production descriptors provide ordinary failure maps into the maintained root contract. That
contract must support the checked-add failure while retaining its input, arithmetic cause, and prior
facts; update the production contract coherently if needed. The test must not add a mapper to make
the inserted State fit. Framework authors can still select custom maps when expressing new semantics.
No separate `Then` algebra or nested sequence-failure representation is prescribed by this design.

Runtime retains the original execution failure independently of root projection. Operational failures
can have no mapped domain root. Overflow, mapping, or report-capacity failure must not discard the
acknowledged original, deployment facts, or pending authority. Checked ABI result access must verify
the selected getter, observation calldata, and exact return contract, not just cast returned bytes.

## Execution and checked results

One public Runtime owns preparation, execution, bounded waiting, and checked result delivery.
There is no separately constructed public driver. A builder makes explicit dependencies and checked
options visible and validates construction; per-run identity, workflow, and initial input belong to
`execute`. Configuration loading remains outside deterministic State execution and the generic
Runtime does not parse EVM-specific documents.

Application and library callers use this same execution surface. Direct start/progress/read/resume
access remains available for deliberate control and boundary cases. Internal waiting invokes the
existing transition owner; it is not another engine. Attempt budgets cannot alter admitted recovery
allowances or authorize replacement commands.

Deadline expiry is a stopped invocation, not a manufactured durable failure. It retains the exact
RunId and any last qualified observation. That observation can predate an acknowledged append whose
projection failed: the outcome must preserve known acknowledgement/authority separately and must
not label an older observation the latest acknowledged head. Cancellation and unavailable observations
must remain honest about what was actually observed or acknowledged.

Runtime follows the [shared action contract](../RFC_RESHAPING_PUBLIC_FACING_N_TESTS.md#concrete-architect-designs):
`RecoveryStopped` and invocation/Store failures end automatic driving. Ordinary pending polling
reconciles only the same retained command. It must not silently repeat ambiguous admission/appends
or replace pending authority. Cold continuation uses the same RunId and checks retained Program identity; exact schema checks govern
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
and block hash/number equal to the configuration receipt. Validation records agreement with the
effective argument, and the checked report value equals `84`. The configuration command encodes `84`,
while
the admitted input still contains `42`. The added Pure transition creates no transaction or nonce.

A separate node query checks deployed code and receipt identity. Its independently decoded expected
result must not reuse the workflow's own report construction to prove correctness. Fresh reservations
use nonces `0` and `1`, and external pending nonce becomes `2`. Assertions retain the returned facts;
they do not reconstruct the transaction protocol inside the test.

The test owns component order, configuration selections, RunId, execution policy and expected
answers. Production owns context/recipe types, checked arithmetic, ordinary failure maps, exact
association, ABI support, execution and result access. Fixture code owns services, funding, independent observations and explicit fault injection. Fault controls
must not become production operation options.

## Case runs and replacement coverage

| Named case | Status and evidence |
| --- | --- |
| `recomposes_with_checked_addition` | New baseline; reuse the maintained operation's components with one production Pure State inserted, and independently observe `84`. |
| `maintained_operation_uses_same_components` | Required comparison in an isolated fresh signer/nonce domain: the unchanged maintained operation and configuration produce `42`; no fixture copy of its implementation. A fresh RunId alone does not reset nonces. |
| `standalone_deploy_uses_same_injection` | Required consuming coverage: select Deploy directly, retaining the same validation and complete injected transaction sequence without an Operation wrapper. |
| `validation_rejects_observed_value_mismatch` | Required focused coverage: a mismatching observation produces typed lifecycle failure; Report must not produce a success artifact. Intrinsic evidence checks remain with their existing owners. |
| `addition_overflow_stops_before_configuration` | Required boundary coverage: checked overflow retains its cause and preceding deployment facts; no configuration transaction is prepared or submitted. A focused test may own this guarantee. |
| `lost_reservation_ack_resumes_same_command` | Preserve committed reservation acknowledgement loss and Runtime reconstruction from the existing Effect E2E. Keep the same keystore owner alive; do not claim process-restart key recovery. |
| `external_nonce_advance_affects_only_fresh_transaction` | Preserve independent external transfer, then a fresh ordinary call reserving nonce `3`; pending nonce moves `2 -> 4`. |
| `cold_terminal_observation_preserves_head_output_nonce` | Preserve exact terminal head/output/nonce under cold read and resume. These assertions do not establish zero provider calls. |
| `authority_sql_failure_preserves_hot_cold_causes` | Preserve actual authority SQL failure and retained layers through cold Application observation; no reservation precedes successful authority access. |
| `retained_epoch_mismatch_is_internal_without_append` | Preserve internal classification and unchanged acknowledged history. |
| `closed_signer_owner_retains_distinct_failure` | Preserve signer cause, classification and unchanged transaction authority through cold observation. |
| Prepared-wire/rejecting-signer recovery, cancellation, ambiguous transaction appends | Preserve existing focused `transaction_tests.rs` ownership unless explicitly relocated with equivalent assertions. |
| Mismatched adjacency, missing binding, wrong typed result, malformed ABI return | Required focused consuming/compile-fail checks for the proposed APIs; no live node required where the owning boundary suffices. |
| Runtime deadline, cancellation and ambiguous acknowledgement | Required focused outcome/authority checks for the Runtime execution surface; additional live case runs are future enhancements. |

The starting assertions live in
[the Effect E2E](../crates/live/evm/tests/evm_contract_effect_e2e.rs), with fixture composition in
the removed `contract_workflow.rs` fixture (available at revision `660285d0`) and focused protocol
coverage in [transaction_tests.rs](../crates/live/evm/src/transaction_tests.rs). Before deletion,
record each old assertion's executable replacement and managed owner. Future cases never justify
removing currently exercised guarantees. The managed `effect-e2e` identity can initially remain while
its selection and documentation change with the executable replacement.

## Complete cutovers and verification

1. Provide the maintained lifecycle, production config/context/result contracts, typed State
   selections, meaningful validation/reporting, and checked-add slot binding. Reuse transaction/Read semantics rather than relocate
   the fixture implementation unchanged. Prove original `42` and recomposed `84` behavior.
2. Improve descriptor inference and executable selection through the current authoring DSL and
   generalize its root to Operations and State selections. Move wrapper-owned validation into the
   checked occurrence/domain owner, then delete unnecessary one-State wrappers, superseded
   registration lists and caller-owned ordinary maps with their replacements. Keep capability-owned
   injection and update authoritative contracts; add no parallel DSL or State expansion hook.
3. Provide one Runtime construction/execution surface and checked result/ABI access. Delete replaced
   fixture drivers/codecs, preserve direct progression, and align Application. Combine inseparable
   API changes rather than leave two designs underneath wrappers.
4. Replace the Effect E2E organization with these cases and mapped existing fault guarantees. Remove
   superseded `EffectFixtureOperation`, caller context/recipe aliases, `register_fixture_states`,
   `DecodeValue`, duplicate report validation, and mechanical failure reconstruction. Retain actual
   fault infrastructure and independent oracles. Update managed selection with the executable cutover.

Validate the current DSL with production descriptors, nested Operations, two supported contexts,
custom maps/handlers, standalone selections, capability injection, and failed expansion. Reject incompatible recompositions
at the strongest supported boundary; use compile-fail tests only where the API guarantees static
exclusion. No caller-defined aliases/maps may be necessary for the target baseline to compile.

Run affected focused tests, managed cases, and final CI according to the
[build guide](build-and-verification.md) for executable changes. This documentation revision needs
link/contract review and `git diff --check` only; production-code LOC change is zero.

The intended simplification removes caller context scaffolding, independent registration, and a
separate public driver and unnecessary one-State wrappers while reusing the existing DSL. Necessary production additions are maintained
component/configuration contracts, checked arithmetic and ergonomic association/execution support.
Implementation commits must report actual additions and deletions.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Production descriptors can infer exact contracts and ordinary maps through the existing DSL. | Current methods expose explicit generic/map parameters; the common constructor and step-selection inference remain proposed. | The sketch could hide a second DSL or leave caller boilerplate intact. | Compile the target caller without structs/aliases/maps through the existing lowering; also exercise nested operations, handlers, injection and failed expansion. |
| A common root entry can qualify Operations and State selections. | Root validation currently lives on Operation, and raw State contracts differ from injected contracts. | Wrapper removal could lose binding/mode checks or expose prepared facts as caller input. | Prove standalone/nested equivalence, invalid-binding rejection, exact expanded/raw contracts and cold decoding without a second expansion owner. |
| Validate and Report have distinct useful contracts. | The production lifecycle value/failure schemas are not yet specified. | Extra States could duplicate intrinsic checks or provide no observable behavior. | Specify effective-value mismatch, checked agreement and report evidence; retain independent assertions and bounded causal failure tests. |
| A reusable typed argument slot can support addition before command construction. | Current input plans already contain encoded calldata. | Arithmetic would require rewriting bytes or retained authority. | Define checked scalar config/context contracts; prove admitted `42` remains unchanged and only the later configuration command contains `84`. |
| The maintained root failure contract can include inserted arithmetic failure coherently. | Its checked schema and mapping ownership are not yet selected. | A test-only adapter or lossy catch-all could return. | Specify overflow, original-cause retention, prior facts, report bounds and cold decoding in production contracts. |
| One Runtime execution surface preserves authority distinctions. | Deadline, cancellation, uncertain admission and post-acknowledgement projection failure differ. | A stopped result could duplicate work or overclaim its observed head. | Validate the shared action matrix and exact recovery identity; distinguish acknowledged authority from historical observations. |
