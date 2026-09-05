# Composition requires too much caller-owned typed plumbing

## Problem

The reusable execution primitives are stronger than the experience of composing them. A developer
can reuse transaction execution, anchored reads, balance collection, Runtime, and persistence, yet
still has to implement States, introduce intermediate value and failure contracts, repeat exact
generic registrations, and write result/progression glue to connect those primitives.

The clearest example is the contract Effect e2e: deploying a contract, calling `configure(42)`, and
reading its value requires a small test-owned application. This proves the extension mechanisms
work, but gives limited evidence that a consumer can assemble the workflow from available product
components. Reusing the execution machinery does not by itself make the workflow reusable.

This review inventories that problem across tests and the production code they exercise. It is a
source review, not a replacement architecture or an implementation change. The current
[design](docs/design.md) and [architecture](docs/architecture.md) remain authoritative.

## Review scope and evidence strength

The review searched Rust sources under `crates/` and `bin/` for State and Operation implementations,
registration calls, failure handlers, generic context types, and composition entry points. It then
traced the relevant test scenarios into Program authoring, Runtime assembly, EVM, Portfolio, and
Application. The inventory groups recurring mechanisms rather than counting every repeated line.

| Test surface | Evidence | Assessment |
| --- | --- | --- |
| [Contract Effect e2e](crates/live/evm/tests/evm_contract_effect_e2e.rs) | `PrepareConfiguration`, `PrepareObservation`, `DecodeValue`, `Abort`, `EffectFixtureOperation`, `runtime`, `WalletCall`, `drive_to_success` | Strongest example of consumer workflow plumbing living in a test. |
| [Generic transaction Runtime integration](crates/live/evm/tests/generic_transaction_runtime.rs) | `TransactionProgram<K>`, two context types, eight State registrations | Demonstrates generic reuse and the repeated work needed to assemble each exact specialization. |
| [Transaction adapter/recovery tests](crates/live/evm/src/transaction_tests.rs) | `TransactionProgram`, `runtime`, four State registrations | Repeats the transaction wrapper and assembly recipe; scripted providers, custody, and append faults are intentional test boundaries. |
| [Runtime contract tests](crates/kernel/runtime/tests/runtime_contract.rs) | `GenericProgram`, `PureProgram`, `ReadProgram`, `EffectProgram`, exact registrations and assembly rejection | Exposes the low-level authoring cost; custom semantics are appropriate for kernel tests. |
| [Runtime injection tests](crates/kernel/runtime/tests/support/injection.rs) | `Choose`, `Choice`, `Project`, `IncrementProgram`, `InjectedProgram`, six State registrations | Illustrates branching, failure, and transitive registration obligations; the synthetic graph is intentional. |
| [EVM balance unit tests](crates/domains/evm/src/lib.rs) | `OneRead`, `balance_authoring_contracts_remain_composable` | One Read needs a local Operation wrapper; the collection reuses public `CollectEvmBalances<Continuation>`. |
| [Portfolio planning tests](crates/domains/portfolio/tests/planning_contract.rs) | `plan_snapshot` | Positive example: production planning owns the graph and admitted input. |
| [Application Portfolio tests](crates/app/tests/portfolio_runtime.rs) | `application_with_backend`, `ComposedRuntime::compose`, `Application` use cases | Positive example: no test-owned States or graph; test implementations substitute external boundaries. |
| [CLI/REST execution e2e](bin/rest-api/tests/client_execution_e2e.rs) | `generated_rest_run_survives_deletion_and_matches_fresh_cli_execution` | Positive example: config and production binaries drive the same product workflow. |
| [EVM contract tests](crates/domains/evm/tests/anchored_call_contract.rs), [transaction contracts](crates/domains/evm/tests/transaction_contract.rs), [read contracts](crates/domains/evm/tests/read_contract.rs) | Production constructors, State contracts, context preservation, exact evidence/wire checks | Useful component coverage; does not establish the ease of composing a complete workflow. |

Program graph/authoring tests, derive compile-fail fixtures, and the remaining foundation,
storage, config, signing, keystore, and provider tests were screened for the same composition
patterns. Hostile wires, synthetic schemas, providers, and Store implementations are not evidence
of missing reusable application States merely because they are custom code.

## 1. Connecting existing components requires new executable States

In the [Effect fixture](crates/live/evm/tests/evm_contract_effect_e2e.rs), `PrepareConfiguration`
takes a deployment completion, extracts the created address, builds calldata, selects fees and gas,
and constructs `EvmTransactionContext<Deployment>`. `PrepareObservation` extracts the call target
and receipt anchor, then constructs `AnchoredContractCallContext<Configuration>`.

These are transitions between already reusable EVM components. Each transition still needs a
named type, `State` implementation, implementation ID, exact input/output/failure contracts,
`PureState` implementation, graph occurrence, and Runtime registration.

The transformation is necessary: the next command depends on a result obtained during execution.
An Operation expands before execution and cannot inspect that result. Current design explicitly
assigns these projections to caller-owned Pure States and excludes action-specific bridges from
the EVM domain. The test follows that contract.

The usability problem is the amount of machinery exposed for supplying the transformation, and
the absence of a production composition for this workflow. Moving the same hardcoded fixture
States into a source directory would not by itself demonstrate general reuse.

Portfolio shows the same boundary in production. In
[its domain implementation](crates/domains/portfolio/src/lib.rs), `EnterPortfolioCollection` builds
an `EvmBalanceContext<PortfolioContinuation>` and `ResumePortfolioCollection` consumes the child
completion. The latter also validates ordinals, converts holdings, and checks totals: those are
real domain responsibilities, so classifying the entire State as removable boilerplate would be
incorrect. The common issue is that entering and leaving a child Operation requires explicit
executable adaptation.

## 2. Contract encoding and decoding are intertwined with composition scaffolding

The Effect fixture contains selectors, `fixture_configure_calldata`, `abi_word`, and
`decode_fixture_value`. `DecodeValue` wraps the getter decode in another registered Pure State,
including its own identity and failure contract.

Knowledge of the fixture ABI is legitimate contract-specific behavior. Generic transaction and
anchored-read States cannot infer the meaning of arbitrary calldata or return bytes. However, a
consumer evaluating this example must implement both that behavior and the framework scaffolding
around it. The test does not demonstrate a reusable contract-call composition that accepts the
contract-specific behavior through a supported production interface.

The anchored-call contract tests reuse `ReadAnchoredContractCall<K>` directly and prove that it
returns checked, context-preserving results. They stop before composing those bytes into a useful
typed application result. That distinction limits the usability claim these tests can support.

## 3. Failure conversion becomes a separate graph and registration concern

`FixtureFailure` unifies deployment reversion, configuration reversion, observation failure, and
local preparation/decode failures. Three `From` implementations perform the external failure
conversions, but those conversions alone do not connect the graph.

The fixture also supplies `Abort<I, O>`, its `State` and `PureState` implementations, three exact
registrations, and three `with_failure_handler<E, J>` regions. `O` describes a success value that
`Abort` never produces; different handler positions nevertheless require concrete output ABIs.
A reader has to understand the protected region, its join contract, the child failure type, and
the root failure type to follow a conversion that ultimately selects one enum variant.

This pattern also appears in production: `PortfolioSnapshotOperation` protects
`CollectEvmBalances`, while `MapEvmBalanceFailure` converts the EVM failure into
`PortfolioSnapshotFailure`. The State declares `PortfolioSnapshotOutput` as its success contract
even though its evaluator always returns failure.

The [authoring implementation](crates/kernel/program/src/authoring.rs) explains the burden:
`with_failure_handler<E, J>` handles an exact failure contract and requires a State-first handler.
It is an executable recovery region, so simple failure propagation/adaptation is expressed using
the same machinery as substantial recovery behavior.

Typed failure semantics and redaction remain valuable. The problem is the repeated adapter State
and handler construction required to preserve them. Runtime, adapter, and Store errors remain
separate from these durable domain failures.

## 4. Preserving context multiplies the exact types a caller must track

The Effect fixture defines:

```rust
type Deployment = EvmTransactionCompletion<EvmU256>;
type Configuration = EvmTransactionCompletion<Deployment>;
```

Those choices propagate through transaction input, completion, reversion, observation context,
observation failure, and handler registrations. The second transaction carries the full deployment
completion as its caller context; the observation carries the nested configuration completion.

The [generic transaction test](crates/live/evm/tests/generic_transaction_runtime.rs) deliberately
proves that `FirstContext` and `SecondContext` can use the same implementation identity with
different exact schemas. This is successful generic reuse, but the caller still supplies the
complete registration set for each specialization. A new context shape changes multiple use sites.

Portfolio uses a named `PortfolioContinuation` to carry aggregation state instead of nesting a
transaction history. Its enter/resume States still own packing and unpacking that continuation.
Context preservation solves retention; it does not automatically solve context selection,
projection, or assembly.

Nested context can also increase retained value sizes because these completions serialize their
caller context. This review establishes the structural nesting, not a measured performance or
capacity defect. It does not assume every workflow needs to retain every preceding completion.

## 5. Injection hides graph expansion but exposes its implementation inventory

Authoring one `ExecuteEvmTransaction<K>` occurrence expands to four States through
[capability injection](crates/domains/evm/src/transaction/stages.rs):

1. `ReserveEvmNonce<K>` Effect.
2. `PrepareEvmTransaction<K>` Effect.
3. `ExecuteEvmTransaction<K>` Effect.
4. `ProjectEvmTransactionOutcome<K>` Pure State.

Every transaction Runtime fixture manually registers that complete set. The recipe appears in
`transaction_tests.rs`, twice for different contexts in `generic_transaction_runtime.rs`, and
twice in `evm_contract_effect_e2e.rs`.

The main Effect fixture's `runtime` helper has 15 explicit State registration calls: eight
transaction specializations, one anchored Read, three local transformation States, and three
failure adapters. This count describes the current assembly; it is not a proposed regression
assertion on implementation size.

`register_evm_transaction_adapters` packages the three IO callbacks, but intentionally registers
no States. `RuntimeAssemblyBuilder::register_effect` registers the named State and its ABI, not
the transitive States inserted by the authoring injection policy. Thus the authoring abstraction
and the consumer's assembly obligations have different levels of detail.

The generic injection test exhibits the same issue without EVM: the caller registers `Increment`,
`Project`, `Choose`, `Observe`, `Mutate`, and `Injected` even though the root uses a composed
injected occurrence. This makes transitive implementation knowledge a general composition cost.

Exact association correctly rejects incompatible assemblies. The concern is maintenance across
separate graph and registration sites, not evidence that an incorrect assembly executes silently.
The Runtime test `missing_effect_adapter_is_rejected_before_store_io` explicitly preserves safe
rejection when a required adapter is absent.

## 6. Even a single existing State needs a root Operation wrapper

Several tests define an Operation whose expansion is one call:

| Wrapper | Body |
| --- | --- |
| `TransactionProgram<K>` in generic transaction integration | One transaction Effect occurrence. |
| `TransactionProgram` in transaction recovery tests | The same occurrence specialized to `EvmU256`. |
| `WalletCall` inside the Effect e2e | The same occurrence for the external-nonce follow-up. |
| `OneRead` in EVM balance unit tests | One `CheckChainIdentity` Read. |
| `PureProgram`, `ReadProgram`, `EffectProgram` in Runtime tests | One synthetic State occurrence each. |

`expand_program` accepts an `Operation`, so each wrapper declares input, output, failure, setup
storage where needed, and an expansion method. The generic transaction wrapper also carries
`PhantomData` and a constructor. The root abstraction has a purpose, but repeated one-step wrappers
show the minimum ceremony consumers pay before they have authored any meaningful composition.

`CollectEvmBalances<K>` is a positive contrast: consumers can pass an existing public Operation to
`expand_program`. The transaction examples do not have an equivalent shared wrapper in use.

## 7. Branching and joins require explicit selector and contract plumbing

The Runtime injection fixture defines `Choice`, a `Choose` Pure State, stable `left`/`right` arm
selectors, explicit arm payload types, and `match_join` contracts. It separately implements
`Project` and an outer failure handler. Production `CollectEvmBalances<K>` similarly composes
`SelectBalanceAsset<K>`, `EvmBalanceAsset<K>`, named native/token arms, and a shared context join.

These are valid deterministic graph semantics. The fixture is specifically testing injection and
recovery, so replacing its custom States with EVM States would obscure its purpose. Its relevance
is the number of concepts needed to express value selection and continuation.

The API uses concrete Rust types, but `OperationExpansion<I, O, F>` is not a type-changing builder
for each intermediate edge. The private draft checks the current contract and the exact joins
during expansion. Many mistakes therefore surface as `ProgramError::InvalidContract`, while
assembly incompatibility uses `RuntimeError::IncompatibleAssembly`. This is an ergonomics and
diagnostic concern supported by the API shape, not a claim that every connection is unchecked or
that a particular test currently fails.

## 8. Typed authoring returns to manually decoded results

The Effect e2e's `terminal_value` matches `RunViewState::Succeeded` and deserializes canonical bytes
into `EvmU256`. `drive_to_success<F>` independently takes a failure type parameter and deserializes
failed values into it. Its generic bound is `DeserializeOwned + Debug`; it is not tied to the
Operation's failure contract by a typed run handle.

The generic transaction test also manually matches success views and checks contract references
and canonical bytes. Those exact assertions are appropriate for its codec test, but illustrate
the boundary: [Runtime](crates/kernel/runtime/src/lib.rs) exposes a heterogeneous `RunView` with
`RetainedValueView`, rather than carrying the authored Operation's result types through that API.

A heterogeneous read-by-RunId surface is necessary for inspection. The consumer cost is recovering
already-known result types manually when executing a known typed workflow. Contract/wire tests
and transport JSON tests should retain their independent wire assertions even if that experience
improves.

## 9. Driving a composition adds caller-owned lifecycle logic

`drive_to_success` distinguishes Runnable, Unavailable, terminal domain failure, success, and
unexpected Runtime errors, with a deadline and retry interval. The fresh wallet run separately
tries resume before start so ambiguous admission does not cause an unsafe assumption about genesis.
The helper even has its own regression test,
`progress_retries_unavailable_then_reports_typed_failure_immediately`, using a local
`FailingOperation` and `Abort<FixtureFailure, EvmU256>`.

Some of this is intentionally specific to fault injection and bounded test execution. Runtime is
caller-driven by contract, and retry/deadline policy cannot be inferred universally. This is an
adjacent usability cost rather than proof that polling belongs in Runtime. It does mean a working
composition example teaches custom lifecycle policy in addition to graph authoring and assembly.

Application provides production start/progress/read use cases and typed recovery information;
the Portfolio application tests exercise those APIs. The transaction e2e currently operates below
that packaged product surface.

## 10. Packaged product reuse is uneven

Portfolio planning tests call `plan_snapshot`; Application tests use `ComposedRuntime::compose`;
the transport e2e supplies config to actual binaries. Their test-specific code primarily describes
inputs, external boundaries, faults, and assertions. They demonstrate the desired separation
between consuming a workflow and implementing it.

That packaging is specific to Portfolio. The
[Application component table](crates/app/src/inspection.rs) contains 13 State registrations for
Portfolio and its `PortfolioContinuation` EVM balance specialization, plus one public reusable
Operation inventory entry. Its registration function is private to Application. Production
`ComposedRuntime` deliberately excludes transaction Effects and anchored transaction-route Reads.

Consequently, a public generic Operation and its successful compilation do not establish a
complete reusable execution package for another caller context. The balance authoring test proves
`CollectEvmBalances<Continuation>` expands, while the application execution tests cover the
Portfolio specialization. A consumer starting outside that product still has to solve assembly.

This is not evidence that transaction execution should immediately be exposed through CLI/REST:
the documented transaction settlement scope remains the pinned non-reorging development fixture.
Library composition usability can be evaluated independently of production transport admission.

## Consequences for design evaluation

- **Multiple change sites:** changing a context or expanded component can require edits to States,
  handlers, wrapper Operations, registration lists, and output decoding.
- **Large entry cost:** the smallest transaction examples already need wrappers and a transitive
  registration recipe; a composed workflow adds substantially more concepts.
- **Test-local product behavior:** the Effect e2e can pass while the deploy/call/read workflow is
  unavailable as a reusable production composition.
- **Uneven discoverability:** product inventory helps with the compiled Portfolio surface but does
  not teach a new caller how to assemble every public generic EVM component.
- **Review burden:** domain choices, mechanical type adaptation, execution assembly, and recovery
  policy compete with the scenario's assertions for the reader's attention.

These findings concern usability and maintenance. They do not establish broken determinism,
incorrect nonce recovery, unsafe evidence binding, or faulty persistence.

## What an improvement should demonstrate

1. A consumer-facing deploy/call/read scenario uses public production components for supported
   workflow behavior. Test-local code expresses fixture ABI/data, injected faults, and assertions;
   any remaining custom State has an explicit domain or extension-testing reason.
2. Using a composed component does not require callers to duplicate its transitive State inventory
   in each test or application. Required IO capabilities remain explicitly supplied.
3. Changing caller context has a small, identifiable authoring/assembly surface while preserving
   exact generic schemas and State ABIs, including hot execution and cold reload.
4. Simple success projection and failure conversion can be distinguished from substantial domain
   execution/recovery. Improvements reduce repeated definitions and change sites, rather than
   hiding the same obligations in test helpers.
5. A known typed workflow has a clear path from input to typed terminal outcome. Heterogeneous
   inspection and exact wire verification remain supported.
6. At least one integration scenario demonstrates reuse outside the fixed Portfolio composition;
   authoring-only success is insufficient evidence of executable reuse.
7. Kernel extension tests retain synthetic States, and fault tests retain hostile external
   boundaries where those are necessary to prove the contract under test.

Any implementation must preserve immutable Program authoring, deterministic State execution,
Runtime's sole fold, exact association and evidence binding, explicit IO authority, append-only
history, redaction, and crate dependency direction. Adding a production bridge, changing failure
authoring, or connecting authoring to assembly are separate architectural choices that require a
target design and complete cutover scope before implementation.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| The desired reusable surface includes deploy/call/read workflows for library consumers. | The user's usability goal is clear, but the supported range of contract ABIs and workflow policies is not specified. | A narrowly packaged fixture could be mistaken for a general solution, or the change could grow into an unnecessary ABI framework. | Define representative consumer scenarios and identify which behavior is supplied as data versus developer code before selecting a target design. |
| Repeated registration and adapter definitions are meaningful developer costs. | Duplication is directly observable; development time and omission frequency were not measured. | The eventual remedy could add more conceptual machinery than it removes. | Compare complete consumer examples and change sites before/after, including a new caller context and a changed child composition. |
| Nested context has a potentially meaningful retained-size cost beyond this fixture. | Serialization structure is visible, but this review ran no size or performance experiment. | Treating it as a demonstrated capacity defect would overstate the evidence. | Measure canonical value/frame sizes on representative composed workflows if size becomes a design criterion. |

Current ownership is explicit in the design documents. This report identifies costs within those
contracts; it does not infer an undocumented owner or select a replacement design.

## Verification

This change is documentation only. Verification consists of checking cited paths and symbols,
reviewing the report against the current code and contracts, and `git diff --check`. No Rust tests,
managed e2es, benchmarks, or broad CI are required for this source-review artifact under
[build and verification](docs/build-and-verification.md).
