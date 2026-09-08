# Reusable production responsibilities implemented by the E2E harness

## Problem

The EVM contract Effect E2E consumes production transaction execution, observation, keystore,
and persistence primitives, but still implements much of the software needed to turn those
primitives into a usable workflow. A consumer must learn and reproduce graph assembly,
failure adaptation, progress policy, and result decoding in addition to choosing domain behavior.

The intended consumer experience is:

1. Obtain and fund a wallet.
2. Import its key and supply explicit execution capabilities.
3. Select the workflow and provide its input.
4. Execute under an explicit progress/deadline policy.
5. Inspect a typed terminal report.

This includes assembling a new workflow from reusable components. Defining new business behavior
can require authoring code; connecting existing components should not require reimplementing
their execution support. The benchmark concerns the complete consumer implementation, including
support modules, rather than the number of lines in the final test function.

The gap is not that tests contain structs or helper functions. It is that reusable responsibilities
have no supported production entry point, forcing tests to implement them. Moving unchanged code
into a helper or promoting hardcoded fixture behavior into a library does not resolve that gap.

## Scope and evidence

This problem statement follows the source review of commit `b1c26a19`, which completed
the [accumulating-context cutover](docs/rfc-ctx-acc-implementation.md). At document preparation, HEAD was `836dc2a1` and another session
was changing implementation details, including failure-report representation. The findings concern
the responsibilities and consumer obligations below; they do not depend on the earlier report
type names, exact line counts, or preserving that representation. Concurrent cleanup is outside
this document's review scope.

The complete harness includes:

- [Managed E2E](crates/live/evm/tests/evm_contract_effect_e2e.rs): resource setup, funding,
  acknowledgement-loss injection, Runtime construction, execution, external nonce advancement,
  recovery, and assertions.
- [Workflow support](crates/live/evm/tests/support/contract_workflow.rs): named contexts, recipe
  selection, graph authoring, ABI/report semantics, failure adaptation, and State registration.
- [Deterministic coverage](crates/live/evm/tests/support/accumulating_contract.rs): scripted
  external evidence, recording Store, failure branches, and cold terminal checks.
- [Capacity coverage](crates/live/evm/tests/support/context_capacity.rs): another context shape,
  an additional creation, failure reporting, and schema/object/frame/run measurements.
- [Managed task](nixfied.nix): PostgreSQL/Reth resources, pinned Solidity compilation, and test
  selection. [The build guide](docs/build-and-verification.md) describes its guarantees.

Production boundaries examined include [Runtime](crates/kernel/runtime/src/lib.rs),
[Program authoring](crates/kernel/program/src/authoring.rs),
[EVM transaction stages and Operation](crates/domains/evm/src/transaction/stages.rs),
[State-family registration](crates/live/evm/src/assembly.rs),
[JSON-RPC provider](crates/live/evm/src/json_rpc.rs),
[keystore](crates/keystore/src/lib.rs), and
[Application composition](crates/app/src/lib.rs).

This is a problem inventory and a set of improvement criteria, not a replacement architecture.
[Design](docs/design.md) and [architecture](docs/architecture.md) remain authoritative.

## Relationship to accumulating contexts

The RFC moved checked plans, cumulative facts, typed field replacement, creation-dependent command
selection, the one-transaction Operation, and four-State transaction registration into production.
The E2E no longer implements the two mechanical preparation bridge States.

However, the RFC explicitly excluded general assembly registration, retry policy, heterogeneous
result inspection, and general failure authoring. It explicitly retained product ABI decoding,
checked terminal failure reports, failure handlers, and a bounded progression driver.

Consequently, completion of that cutover does not establish complete reusable workflow support.
The remaining costs below are not
automatically stale implementations or failures of that cutover. Several are limitations of its
deliberately bounded scope.

## Inventory against the five-step consumer flow

| Consumer step | Implementation currently supplied by the harness | Reusable production responsibility missing or insufficiently packaged | What remains consumer/test-owned |
| --- | --- | --- | --- |
| Obtain a wallet | `generated_signer`: entropy, checked scalar construction, retries, purpose selection, import | A reusable wallet/key-creation entry point if obtaining a new wallet is a supported platform use case; checked import already exists | Choosing an ephemeral wallet, its purpose, and lifetime |
| Fund a wallet | `fund_sender`, `funding_rpc`, `funding_response_body`, `FundingResponse<T>`, `FundingError` | Reusable bounded JSON-RPC transport mechanics; a funding capability requires an explicit development/faucet authority contract | Selecting the funding source, amount, development node, and readiness requirement |
| Supply capabilities | `runtime`: opens Store and authority, constructs provider, installs State and adapter families | A reusable composition boundary accepting explicitly supplied dependencies and the selected workflow's executable requirements | Locators, signer/authority selection, fault wrappers, resource lifetime, and deliberate reconstruction |
| Select workflow/input | `EffectFixtureOperation`, context/recipe aliases, `register_fixture_states` | Composable executable components whose required State inventory is maintained with their implementation | Step ordering, selected source fields, plans, fees, ABI, and product policy |
| Connect failures | `Abort`, its trait implementations and registrations, `with_failure_handler`, `TryFrom` conversions | Reusable typed failure adaptation without handwritten executable scaffolding for every mapping | Which failures are terminal, their reviewed reasons, and genuine recovery behavior |
| Retain terminal facts | `FixtureFailure` construction/decoding and context-to-report conversions; analogous capacity code | Reusable lossless report construction for supported compositions, with bounded representation and checked contracts | Product names/order, chosen report content, and domain-specific consistency rules |
| Execute | `drive_to_success`, resume-before-start logic for the wallet follow-up, timeout and polling constants | A reusable caller-controlled execution driver with explicit deadline/progress/recovery policy | Policy values, injected failures, expected interruption points, and assertions |
| Inspect report | `terminal_value`, failure deserialization in the driver, manual `WalletReport` decoding | Exact-contract-checked typed terminal access, with a clear relationship to the selected workflow's success/failure types | Expected values and independent wire/history assertions |

The table separates demonstrated framework gaps from the wallet/funding product-surface questions.
The latter do not justify adding ambient key generation or unlocked-account RPC methods to domain
States or broadening production transaction admission merely to shorten a test.

## 1. Transport implementation leaks into funding setup

`fund_sender` constructs a second HTTP client and implements JSON-RPC requests and responses.
It repeats no-proxy/no-redirect/no-retry policy, request deadlines, bounded response collection,
version/ID checks, and redacted error conversion. The production `JsonRpcEvmProvider` already owns
these mechanisms through its private `rpc`, envelope types, HTTP construction, and body reader.

The test needs `eth_accounts` and `eth_sendTransaction`, which the production transaction-provider
port does not expose. That explains the duplicate implementation; it does not make all those
transport mechanisms inherently test-specific.

The reusable portion is bounded transport and protocol handling. Development-node account discovery
and unlocked-account funding are separate authority-bearing behavior. A supported funding component
must make that distinction explicit and define whether it acknowledges submission or actually waits
for funding readiness. The reviewed `fund_sender` consumes a returned transaction hash and returns;
it does not itself wait for a successful receipt or sufficient balance.

`FundingError` is a legitimate redaction boundary. The subsequent LOC reduction shares a generic
`FundingResponse<T>` and one test-only `funding_rpc` request path. This removes the duplicate
envelope and checks within the harness; the broader transport responsibility remains as described
above.

## 2. Wallet creation is implemented by the caller

`generated_signer` obtains random bytes, constructs a checked secp256k1 scalar, retries rejected
candidates, and imports the key with a signing purpose. Production already owns checked secrets,
import, signing handles, and keystore lifecycle; it does not expose the complete creation operation
used here.

If creating a fresh wallet belongs to the supported platform surface, callers should select that
operation rather than repeat secret-generation and import mechanics. Entropy, secret lifetime,
and redacted errors need one reviewed implementation. This is a narrower requirement than adding
a wallet-management framework, mnemonic support, or new key persistence.

The choice to create an ephemeral key and keep its keystore owner alive across recovery remains
test policy. No secret should enter workflow input, Program, Journal, reports, logs, or diagnostics.

## 3. Workflow composition does not carry its executable requirements

`EffectFixtureOperation::expand` selects the graph. `register_fixture_states` independently lists
the implementations needed to execute that graph. The surrounding `runtime` function combines
those registrations with the wallet follow-up family and explicit IO adapters.

The four-State transaction helper is a real improvement: callers no longer list the transaction's
internal stages. The same ownership problem remains at the composed-workflow boundary. Changing
the graph can still require a separate registration edit; the consumer must know its transitive
implementation requirements. Association rejects incompatible assembly, but does not remove the
maintenance obligation that produced it.

Reusable components should expose sufficient composition support that consuming them does not
mean reproducing their implementation inventory. The consumer must still supply Store, signer,
authority, and providers explicitly. Program must remain immutable graph data, with Runtime owning
association and execution; this problem statement does not prescribe merging those owners.

The existing `ComposedRuntime` packages Portfolio. It intentionally does not package transaction
Effects or anchored transaction-route Reads. The E2E therefore cannot obtain this experience by
calling an existing general application entry point.

## 4. Failure conversion requires caller-authored executable machinery

The product's failure conversion is accompanied by `Abort` State declarations, implementation IDs,
exact input/output/failure types, evaluators, graph occurrences, and registrations. Each protected
region also supplies a concrete successful join type even though the aborting handler produces
only failure.

Choosing a root failure reason is domain policy. Repeatedly expressing an ordinary typed conversion
as a custom executable State is framework ceremony. The framework should support this reusable
adaptation while keeping genuine recovery graphs explicit and preserving exact failure contracts,
deterministic identity, redaction, and durable semantics.

This does not justify replacing every failure handler with an implicit conversion. A handler that
performs real recovery has different responsibilities from one that only converts and propagates.

## 5. Terminal reporting still reconstructs accumulated data manually

The ordinary EVM stages preserve siblings through generated slots, but terminal failure conversion
selects fields and constructs another representation by hand. The alternate capacity workflow needs
corresponding reporting machinery. Changing the representation can reduce duplication without
eliminating this consumer obligation.

An unrelated field added to the accumulated context survives stage replacement. Its presence in
the terminal failure report still depends on the converter explicitly retaining it. In particular,
adding a new sibling outside the existing request object is not automatically covered by converters
that select only the original fields. This is an extension hazard, not evidence that the existing
fixture has already dropped a required fact.

Reusable report support should preserve the declared facts across supported success and failure
shapes without each consumer rebuilding the same stage bookkeeping. Products still own which
facts their report promises, step names/order, ABI meaning, and business consistency requirements.

Any improvement must remain within schema identity, object, frame, and run limits. Simply embedding
the full context separately in every failure variant can exceed the schema budget; discarding facts
or adding an implicit Journal-backed lookup would change the requirement instead of satisfying it.

## 6. Execution policy and admission recovery are handwritten

`drive_to_success` implements a progress loop, total deadline, polling interval, retries of Runnable
and Unavailable, typed failure decoding, and panic diagnostics. The wallet follow-up separately
tries `resume` before `start` so an unavailable admission acknowledgement does not imply that
genesis was absent.

These are reusable execution-client responsibilities. Consumers should supply explicit policy and
receive structured outcomes rather than copy a panic-based test driver. The supported behavior for
Absent, admission conflict, Indeterminate, Unavailable, cancellation, and deadline expiry needs a
reviewed contract; the current helper is not a general policy for all Runtime errors.

Runtime remains caller-driven. A reusable driver must call the existing authoritative execution
operations, preserve the caller's RunId and exact admission, and leave acknowledged prepares
recoverable. It must not introduce a second semantic fold, silently choose infinite retries,
manufacture terminal domain failures, or assume a timed-out append did not commit.

Repeated Runtime/database reconstruction is deliberate recovery coverage and remains test-owned.
The production driver need not reconstruct every dependency on each poll just because this
particular test does.

## 7. Typed workflow execution returns to manual JSON decoding

`terminal_value`, the failure arm in `drive_to_success`, and wallet-report inspection all select a
`RunViewState` branch and deserialize canonical bytes themselves. The expected output and failure
types were known during authoring, but the consumer has to supply them again after execution.

A reusable typed terminal interface should verify the requested exact value contract before
decoding, distinguish success/failure/nonterminal progress, and return reviewed errors. Successful
Serde decoding alone does not establish that the caller selected the correct nominal/exact
contract: different contracts can have compatible JSON shapes.

Raw retained bytes, contract references, and heterogeneous inspection remain necessary. A typed
accessor can coexist with them. It is one useful improvement, but is not by itself a complete typed
execution interface tied to a chosen workflow.

## 8. Business behavior and test boundaries must not be mistaken for missing framework code

| Harness code | Responsibility that should remain outside generic production execution |
| --- | --- |
| `ReservationAcknowledgementFault` | Commits through the real authority and deliberately loses one acknowledgement. This is the fault being tested. |
| `RecordingStore`, `ScriptedEvidence`, `Fault` | Substitute or observe explicit external boundaries to test behavior. |
| `external_wallet_transfer` | Acts outside Program and nonce custody to represent another wallet application. Replacing it with an MFM-managed transaction would weaken that test condition. |
| `fixture_initcode`, managed Solidity compilation, environment selection | Supply a particular first-party fixture artifact and managed resources. The generic framework does not need knowledge of this file or compiler invocation. |
| `CONFIGURE_SELECTOR`, `VALUE_SELECTOR`, `fixture_configure_calldata`, `abi_word`, `decode_fixture_value` | Describe this contract's ABI and expected semantics. Reusable encoding machinery may support them, but cannot infer the product meaning. |
| `ContractWorkflow`, `FixtureRequest`, recipe source choices, `EffectFixtureOperation` step ordering | Define the chosen product. If exposed as a supported reusable product, it needs a real product contract; copying the hardcoded fixture into EVM core is insufficient. |
| `DecodeValue` and report-specific validation | Perform real ABI/business interpretation. Reduce surrounding authoring ceremony without removing the semantics. |
| Capacity workflow and hostile report inputs | Exercise alternate composition and rejection boundaries, rather than represent a second production execution engine. |
| Nonce, retained command/evidence, head, terminal value, and cold replay assertions | Establish observable outcomes. Assertions and expected values belong to the tests. |
| `nonzero` and ordinary input construction | Express checked fixture constants and inputs; their mere presence does not justify new public APIs. |

External transfer code uses signing/encoding primitives to construct an independent actor's input.
This is not sufficient evidence that MFM needs a second public custody-bypassing transaction API.
Likewise, development funding does not authorize widening the production Effect provider port.

## What completion of this problem should demonstrate

1. A complete consumer example follows the five-step flow using supported reusable APIs. Review
   every supporting module, not only the final test body.
2. A second workflow/context can reuse the same components without copying their transitive State
   inventory, execution driver, or terminal decoding implementation.
3. Adding an unrelated retained field and adding another transaction have small, identifiable
   authoring surfaces; success and failure reports retain their declared facts.
4. Ordinary failure conversion does not require a new handwritten adapter State plus registration
   for each mapping. Genuine recovery behavior remains explicit.
5. Deadline, retry, admission recovery, and cancellation behavior are explicit and reusable, and
   preserve exact RunId/command/history guarantees under ambiguous outcomes.
6. Typed terminal access rejects a wrong exact contract even when its JSON shape would deserialize.
   Independent raw wire/history inspection remains available.
7. Wallet creation and funding use reusable capabilities where admitted as supported product
   behavior; secret custody, funding authority, and readiness semantics remain explicit.
8. Funding does not maintain a parallel implementation of shared bounded RPC mechanics. Any
   extraction reduces total implementations and change sites without exposing an unrestricted
   authority surface merely for convenience.
9. Managed acknowledgement loss, rejecting-signer prepared-wire recovery, cancellation, ambiguous
   appends, external nonce advancement, cold terminal stability, failure facts, and capacity tests
   retain their actual guarantees. A short success example does not replace adversarial coverage.
10. Improvement is measured by removed caller obligations, duplicated implementations, public
    concepts, and future change sites. Test-file movement and renamed wrappers alone do not count.

These criteria require a subsequent target design before implementation. They do not authorize
changes to Program wire, Journal/Store ownership, keystore threading, transaction finality, or
CLI/REST admission. No architecture cutover is selected by this document.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Fresh wallet creation should be a reusable platform capability. | The desired flow includes obtaining a wallet, but supported creation/import sources and persistence policy are not specified. | A convenience API could unnecessarily widen secret-handling responsibilities. | Specify the ephemeral creation use case, entropy boundary, and key lifetime before selecting an API. |
| Funding needs a reusable capability in addition to shared RPC mechanics. | The demonstrated funder is a managed development node with unlocked accounts, not a general production funding source. | Promoting it indiscriminately could introduce inappropriate authority into production execution. | Define the development/faucet boundary and submission-versus-readiness contract; retain explicit authority. |
| A common composition/reporting API can reduce the remaining obligations. | Different workflows have different failure policies, schemas, and supported capacity. No replacement API has been implemented or measured here. | A general abstraction could introduce more concepts or exceed limits while only hiding existing bookkeeping. | Compare complete implementations for the current workflow and an extended context/transaction scenario, including failures and capacity. |
| Responsibility findings outlast concurrent cleanup. | Another session was changing the harness and report representation during documentation. | Specific examples may be removed even though some broader gaps remain, or a gap may be resolved. | Recheck cited symbols and complete consumer change sites against the implementation selected for the next design. |

## Verification

Documentation-only source review: inspect the cited symbols and local links and run a scoped
`git diff --check`. This document changes no executable behavior and selects no Rust tests,
managed E2E, or CI run under [build and verification](docs/build-and-verification.md).
