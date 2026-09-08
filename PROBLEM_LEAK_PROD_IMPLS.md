# Reusable production responsibilities implemented by the E2E harness

## Problem and intended consumer experience

The EVM contract Effect E2E consumes production execution primitives, but implements much of the
software required to assemble and run them. It defines workflow types and failure adapters,
constructs Runtime assembly, manages dependencies and recovery, drives progress, and decodes
results. These are missing reusable production capabilities, rather than unavoidable duties of a
consumer E2E.

The intended consumer experience is:

1. Obtain and fund a wallet.
2. Import its key and supply explicit execution capabilities.
3. Select the workflow and provide its input.
4. Execute under an explicit progress/deadline policy.
5. Inspect a typed terminal report.

Workflow choices and execution policy should be configuration consumed by production code. The
consumer E2E supplies that configuration and checks the resulting report. It should not implement
States, failure conversions, State registration lists, a Runtime factory, a polling/recovery loop,
or report codecs for behavior supported by the platform.

The immediate direction is configuration of supported operations, their order, ABI/function
selection, and static typed inputs. Arbitrary wiring such as “argument X takes output Y from State
Z” is a future capability recorded in [known gaps](docs/known-gaps.md#configuration-driven-workflows).
Existing production connections for created addresses and receipt anchors remain useful; static
input support does not require first inventing a general expression language.

This is a problem statement and agreed direction, not an implemented API or a replacement for the
current [design](docs/design.md) and [architecture](docs/architecture.md). It records missing support
rather than asserting that today's contracts already provide it.

## Scope and evidence

The initial review examined `b1c26a19`, following the
[accumulating-context cutover](docs/rfc-ctx-acc-implementation.md). This revision incorporates the
user's configuration-driven consumer requirements against `14b6141c`. Subsequent cleanup has
changed report representation and reduced duplication, but the consumer obligations below remain.
The findings do not depend on historical line counts or retaining old report types.

The whole harness includes:

- [Managed E2E](crates/live/evm/tests/evm_contract_effect_e2e.rs): resources, funding, acknowledgement
  loss, Runtime construction, progress, external nonce advancement, recovery, and assertions.
- [Workflow support](crates/live/evm/tests/support/contract_workflow.rs): contexts, graph, recipe
  selection, ABI/report semantics, failure adaptation, and State registrations.
- [Deterministic coverage](crates/live/evm/tests/support/accumulating_contract.rs): scripted external
  evidence, recording Store, failures, and cold terminal checks.
- [Capacity coverage](crates/live/evm/tests/support/context_capacity.rs): another context shape,
  additional creation, failure reporting, and schema/object/frame/run measurements.
- [Managed task](nixfied.nix): PostgreSQL/Reth resources, pinned Solidity compilation, and test
  selection. The [build guide](docs/build-and-verification.md) states the actual guarantees.

The production boundaries include [Runtime](crates/kernel/runtime/src/lib.rs),
[Program authoring](crates/kernel/program/src/authoring.rs),
[EVM transaction States and Operation](crates/domains/evm/src/transaction/stages.rs),
[State-family registration](crates/live/evm/src/assembly.rs),
[JSON-RPC provider](crates/live/evm/src/json_rpc.rs), [keystore](crates/keystore/src/lib.rs), and
[Application composition](crates/app/src/lib.rs).

The accumulating-context work made checked plans, cumulative facts, typed replacement, selected
creation/call/observation connections, and transaction-family registration reusable. Its bounded
scope did not solve general composition, execution policy, failure authoring, or typed result
consumption. Completing that cutover therefore did not complete this consumer experience.

## Responsibility inventory

| Consumer step | Implementation leaking into the harness | Required reusable production behavior | Consumer E2E supplies |
| --- | --- | --- | --- |
| Obtain/import wallet | `generated_signer`: entropy, scalar construction, retries, purpose selection, import | Reviewed wallet creation/import and owned key-handle lifecycle | Wallet creation/import request and selected capability reference; no persisted secret in workflow configuration |
| Fund wallet | `fund_sender`, `funding_rpc`, `funding_response_body`, `FundingResponse<T>`, `FundingError` | A `DevNodeFundWallet` State and dev-node adapter, sharing bounded RPC mechanics and defining completed funding/retry behavior | Development-node selection, recipient, amount, and funding options |
| Supply capabilities | `runtime`: opens Store/authority/provider, installs executable and adapter families, rebuilds handles | Production application/composition opens or accepts dependencies, owns their lifecycle, installs selected executable requirements, and constructs the existing Runtime | Explicit deployment/capability selections |
| Select workflow/input | `EffectFixtureOperation`, recipe aliases, ABI helpers, `register_fixture_states`, fixed fee constants | Checked operation configuration, production graph/assembly planning, reusable ABI encoding/decoding, and RPC fee-discovery States | Supported step order, ABI/function selection, static typed arguments, fee policy, and report options |
| Connect failures | `Abort`, its trait implementations/registrations, failure regions and `TryFrom` conversions | Supported failure classes and simple configured terminal/retry behavior, implemented by production components | Failure-class policy; no custom failure-adapter State |
| Retain terminal facts | `FixtureFailure` construction/decoding, context-to-report conversion, analogous capacity code | Production success/failure report construction and validation with complete declared facts | Optional names, report selections, and supported expectation rules as configuration |
| Execute | `drive_to_success`, resume-before-start logic, polling/deadline handling, repeated reconstruction | Production execution entry point and policy driver over the sole Runtime, including supported lifecycle/recovery behavior | Execution configuration; no custom driver or forced interruption in this E2E |
| Inspect report | `terminal_value`, failure/wallet JSON decoding, intermediate head/nonce assertions | Exact-contract-checked typed result and report APIs | Final report expectations only; internal recovery assertions move to focused tests |

The consumer can pass one request containing both operation and execution configuration. Their
meanings differ: operation configuration determines admitted semantics; execution configuration
controls how production code drives that admitted run. Changing a deadline or reconnecting a
provider must not silently rewrite Program, C0, a pending command, or a durable domain outcome.

## 1. Development-node funding should be a reusable State

The target is a small `DevNodeFundWallet` operation/State, selectable like other supported behavior.
It accepts checked public funding input and returns retained funding evidence suitable for the
report. The State prepares a command and interprets evidence; a live adapter performs RPC IO.
Funding mutates external state, so it belongs on the Effect path rather than being disguised as a
Read or executed through ambient IO inside a deterministic State.

A separate general faucet-authority subsystem is not a prerequisite. Selecting and binding an
explicit development-node adapter can provide the required boundary using existing capability
mechanisms. “Production implementation” here means reusable maintained platform code with an
explicit development-node scope, not a promise of general production-chain funding.

The current helper discovers an unlocked account and submits `eth_sendTransaction`, consuming its
returned hash without itself waiting for funding readiness. Moving that call into a State is not
sufficient: duplicate Effect entry after a lost acknowledgement must not accidentally fund twice.
The minimal implementation must define completion evidence, readiness, and recoverable duplicate
handling. The exact dev-node command/replay mechanism remains an implementation design question;
this requirement does not prescribe an additional authority service.

The funding code also repeats HTTP policy, bounded response collection, JSON-RPC checks, and
redacted errors already owned privately by `JsonRpcEvmProvider`. Sharing `FundingResponse<T>` and
`funding_rpc` within tests reduces local duplication but leaves that production responsibility in
the harness. Reusable funding should consume shared transport behavior.

## 2. Wallet creation and dependency lifecycle belong to production

`generated_signer` implements fresh key generation around an existing checked import API. A
consumer should request the supported ephemeral wallet or import a key through production code.
Secret generation, validation, zeroization, import, signer purpose, and owner shutdown should have
one reviewed implementation. This does not require mnemonic support or a new persistence format.

Likewise, the consumer E2E should select dependencies rather than implement their construction and
lifecycle. The earlier phrase “composition boundary” means a concrete production entry point that:

- accepts explicit deployment/capability selections or injected handles;
- installs the selected workflow's exact executable and adapter requirements;
- constructs and uses the existing production Runtime;
- owns normal dependency opening, closing, and supported reconnection behavior.

The test's `runtime(...)` helper is not a second implementation of the Runtime engine: it calls
`Runtime::new`. The missing reuse is the assembly/bootstrap/lifecycle implementation around that
engine. The target removes the test-owned factory and driver, not the use of a real production
Runtime instance internally. Runtime retains the sole semantic fold.

Deliberate teardown and reconstruction for fault verification belong in dedicated integration
coverage. Normal lifecycle and recovery supported for users must be production behavior. The
consumer E2E should not reproduce either policy in a local helper.

## 3. Workflow, ABI, input, and fee choices should be configuration

For supported EVM behavior, consumers should configure an ordered list of operations, selected
ABI/function or selector, static typed arguments, public plan options, and reporting/failure policy.
Production code should validate that configuration, encode/decode ABI values, construct the exact
graph, and install its executable requirements. It should reject incompatible selectors, argument
shapes, and unsupported operations through checked admission, without asking the test author to
write another State or registration list.

The ABI and the intended values remain choices made by the caller, but their implementation should
not remain handwritten fixture Rust. For example, choosing a setter with the static value 42 is
configuration; duplicating calldata construction and a decoder around that choice is not necessary
consumer work. Truly new semantics can require a reusable production component, after which its
supported instances should be configurable.

RPC-derived fees are another missing production operation. A fee-discovery Read State should retain
typed external evidence, and production deterministic code should apply the configured fee rule
and bounds when constructing the transaction command. The configured choice can distinguish fixed
fees from discovery. Once an Effect prepare is acknowledged, recovery must reuse its exact command
and fees rather than query a new price and alter the transaction.

General configuration-driven references from a named prior State output into a later ABI argument
are deferred. The initial scope uses static caller-supplied arguments and supported built-in
connections, including production fee discovery feeding transaction construction. The future
reference feature needs exact type/field checks, dependency ordering, branch availability, and
stable identity; it must not become a dynamic untyped context bag. See
[configuration-driven workflow gaps](docs/known-gaps.md#configuration-driven-workflows).

## 4. Failure classes should be simple configuration

Consumers should select which supported failure classes terminate an operation and which admit
supported retry/recovery behavior. The implementation must not require consumer-written `Abort`
States, `TryFrom` report conversions, failure-handler registration, or a new recovery graph merely
to express ordinary policy.

Production components own reviewed failure classes, classification, and typed propagation. The
configuration selects among their supported behaviors. An unspecified or unsupported policy must
not silently turn failure into success or allow downstream steps to consume absent results.

A terminal domain failure and stopping an execution attempt are different results. An authenticated
reversion can be a durable domain outcome. Unavailable dependencies, cancellation, deadline expiry,
and Indeterminate acknowledgement do not automatically prove a durable failed workflow. Production
policy/result APIs must preserve that distinction while presenting a simple consumer interface.

General recovery is unfinished and is explicitly tracked in
[known gaps](docs/known-gaps.md#general-execution-and-recovery-policy). Existing boundary tests are
valuable evidence for specific guarantees, not a complete configurable recovery product.

## 5. Success and failure reports should be production results

Both success and failure reports should include configured step names/order, public input and plan
information, retained execution/evidence facts, available decoded outputs, failure classes, and the
results of supported consistency/expectation checks. A failure report must describe which steps
completed, failed, or did not run without fabricating missing outputs.

The caller can choose report content and supported expectation rules through configuration.
Production code should construct and validate the report. Names, ordering, and consistency outcomes
are report data, rather than reasons to require test-owned report schemas and converters. Intrinsic
command/evidence checks still belong to their production owners; configured expectations must not
disable them. New business-specific validation can be added as a production component instead of
being reimplemented inside the E2E.

The report boundary should preserve all facts declared by its contract across both success and
failure. Adding an unrelated retained field must not silently lose it in a handwritten failure
conversion. Representation must fit the schema/object/frame/run limits without duplicating whole
prefixes per failure variant or introducing an implicit Journal lookup.

Typed report access should check the exact contract and return structured results. Consumers should
not repeat JSON decoding or reconstruct failure reasons from erased `RunView` bytes. Public result
metadata must distinguish a durable terminal report from a stopped attempt with resumable history;
a deadline must not manufacture a persisted failure report that the workflow never produced.

## 6. Execution should be entirely production-owned

The consumer supplies progress, deadline, polling, and supported retry/recovery configuration to a
production execution entry point. Production code handles admission, resume, dependency lifecycle,
policy enforcement, and typed result delivery. It preserves one caller-supplied RunId, exact
admission and pending command identity, append atomicity, and the existing sole Runtime fold.

The consumer E2E should not contain `drive_to_success`, resume-before-start logic, manually rebuilt
Runtime handles, forced interruption points, or assertions about internal frame progression. Its
assertions compare final report results with the configured scenario's expectations.

Failures and policy choices should be representable through supported configuration. Deliberately
injected infrastructure faults, where needed, can be selected through a separate explicit dev/test
scenario profile backed by reusable test infrastructure. They should not become undocumented
operation semantics or require handwritten wrappers in the consumer E2E. Detailed interruption,
head, nonce, cancellation, and append-boundary verification belongs in dedicated unit/integration
tests. Those tests retain fault implementations and independent assertions; they need not be
limited to a terminal report when the property being proved is an intermediate durability boundary.

Separating these tests must preserve coverage. It is not permission to delete acknowledgement-loss,
prepared-wire recovery, external nonce advancement, cold history, or capacity guarantees.

## What remains outside the consumer E2E implementation

| Existing harness code | Intended destination or treatment |
| --- | --- |
| `ReservationAcknowledgementFault`, `RecordingStore`, `ScriptedEvidence`, `Fault` | Reusable dev/test infrastructure and dedicated fault/recovery integration tests; no local copies in the consumer E2E |
| `external_wallet_transfer` | Dedicated external-nonce integration scenario, retaining an actor outside MFM custody |
| Runtime/database teardown and reconstruction | Production lifecycle where supported for users; deliberate boundary manipulation in dedicated recovery tests |
| Solidity fixture source/compiler task and managed service provisioning | Managed test infrastructure; E2E receives the artifact and deployment configuration |
| ABI selectors, setter values, fee choices, operation order, report selections | Checked consumer configuration interpreted by production components |
| Calldata codecs, `DecodeValue`, graph/registration assembly, failure/report conversion | Reusable production implementation for the supported configurable operations |
| Capacity graphs, hostile report wires, exact head/append assertions | Focused unit/integration/capacity suites |
| Final expected report values | Consumer E2E assertions |

Resource provisioning can remain part of the managed test environment. That does not imply the
consumer test should rebuild production composition or execution behavior. Likewise, independent
fault infrastructure is legitimate testing code, but is not an obligation imposed on ordinary
workflow consumers.

## Completion criteria and sequence

1. A consumer E2E follows the five-step flow using production APIs, configuration, and final report
   assertions only. Review all support modules to ensure no local implementation is hidden there.
2. A reusable `DevNodeFundWallet` State completes configured funding with checked evidence and a
   reviewed duplicate-entry/recovery contract, without requiring a general faucet subsystem.
3. Production composition owns dependency lifecycle and exact executable requirements. A second
   workflow does not copy State registration lists or a Runtime constructor.
4. Supported ABI/function selection, static typed inputs, ordered operations, fee selection, failure
   classes, and report options are configuration. Fee discovery is a retained production Read.
5. Production execution owns progress/recovery policy and typed result decoding. No consumer panic
   loop, ad hoc admission retry, or manual JSON report decoding remains.
6. Success/failure reports retain declared facts, configured names/order, and supported consistency
   results under unchanged capacity limits. Stopped attempts remain distinguishable from durable
   workflow failure.
7. Detailed fault/interruption tests move to named unit/integration scenarios with their assertions
   intact; managed task selection continues to run them where required.
8. Future arbitrary prior-output references and general recovery are tracked explicitly in known
   gaps rather than claimed as already supported by static configuration.
9. Improvements reduce total caller obligations and independent implementations. Moving a fixture
   helper or wrapping unchanged test-only machinery behind a new function is insufficient.

This documentation revision is one coherent change: update this problem statement and known gaps.
Subsequent implementation needs a concrete target API and logical cutovers covering production
components, consumers, authoritative contracts, and tests together. The configuration-driven
consumer direction is selected; exact schemas and APIs are not implemented by this document.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Dev-node funding can fit a small replay-safe Effect contract. | The existing unlocked-account submission does not itself provide duplicate suppression after lost acknowledgement. | A simple RPC wrapper could fund twice or report completion too early. | Select and test the supported dev-node mechanism, retained evidence, readiness, and acknowledgement-loss behavior before implementation. |
| The first configurable ABI/input surface can remain bounded. | Supported ABI types, overload selection, and expectation rules are not yet enumerated. | A supposedly small change could become a general expression/schema engine or leave common inputs unsupported. | Specify complete static setter/getter examples and rejection cases, then validate an additional ABI/context shape. |
| General recovery can be presented as simple class-based policy. | Supported actions and interactions with pending Effects, deadlines, and process/key lifetime are unfinished. | Treating every stopped attempt as terminal failure could misstate history or retry external mutation incorrectly. | Define the supported class/action matrix and test ambiguous admission, cancellation, pending prepare, reconnect, and restart boundaries. |

## Verification

Documentation-only review of cited symbols, local links, requirements consistency, and scoped
`git diff --check`. No Rust behavior, task selection, or executable contract changes in this revision;
no Rust tests, managed E2E, or CI run is selected by the
[build guide](docs/build-and-verification.md).
