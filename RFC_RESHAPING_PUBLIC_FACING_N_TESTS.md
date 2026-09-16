# RFC: reshaping MFM's public interfaces and tests

## Status and scope

The three public usage paths below are the agreed framing for MFM as a platform and framework.
This RFC proposes requirements and acceptance scenarios for improving those paths. Exact Rust
signatures, configuration schemas, executable-registration mechanics, and implementation placement
within the existing crate boundaries remain to be designed. No proposed API is available merely
because it appears here.

[Design](docs/design.md) remains authoritative for execution and persistence contracts;
[architecture](docs/architecture.md) owns current responsibility placement. This RFC replaces the
former E2E production-leak problem statement. It does not change executable or persistence contracts.

## The three public usage paths

These are activities, not mutually exclusive kinds of users. One application developer may do all
three, and an authored composition can become an operation that another caller selects.

| Activity | What the caller should express | What MFM should supply |
| --- | --- | --- |
| Select an existing operation | Its inputs and supported options | The selected implementation's requirements, checked preparation, capability binding, execution access, and checked results |
| Compose existing Operations or States | Their order, valid connections, and any intentional policy | Composition validation, lowering to one Program, association with exact executables, and reusable execution/result handling |
| Implement a new State | Its actual semantics and typed contracts | Pure/Read/Effect contracts, capability integration, canonical value handling, execution, recovery, and persistence machinery |

Selection is the platform experience; composition and extension are framework experiences. All
three are first-class public uses. They share the same Program and Runtime, rather than separate
execution models. A convenient product interface cannot substitute for usable framework APIs.

An Operation is reusable authoring-time composition. Defining one does not necessarily introduce a
new algorithm: it may package existing components into a sequence. A State defines one deterministic
step, including preparation and interpretation for a Read or Effect. Actual IO belongs to an
explicit adapter. Introducing a new external protocol can legitimately require a new capability
and adapter; it is not made safe by hiding IO inside a State.

Using the framework does not imply configuration files. Checked Rust authoring is a public use in
its own right. A bounded configuration-driven product may expose a selected component catalogue,
but arbitrary runtime composition and arbitrary Rust extension are different capabilities.

## Problem

The EVM contract Effect E2E mixes selecting behavior, authoring composition, extending behavior,
constructing live dependencies, driving execution, and testing fault boundaries. This makes it hard
to tell whether a large test is expressing its scenario or compensating for an awkward public API.

The problem is not that tests author workflows. The problem is that using or composing supported
behavior requires callers to implement reusable assembly, execution, and result-handling work.
Moving all fixture Rust behind one product function would hide framework problems. Conversely,
requiring every product consumer to implement States and failure maps would expose implementation
details that the selected product should already own.

Review each responsibility by asking:

1. Is this the caller's scenario, new semantics, or intentional policy?
2. Is this mechanical work needed by every caller of the same supported behavior?
3. Is this deliberate fault setup or an independent assertion needed to prove a boundary?

The first belongs at the relevant public authoring surface, the second needs one reusable owner,
and the third belongs in focused testing infrastructure or the test itself. File location, line
count, and the existence of a custom Operation are not sufficient evidence of leakage.

## Current evidence and existing support

The primary evidence is the [managed Effect E2E](crates/live/evm/tests/evm_contract_effect_e2e.rs)
and its [workflow support](crates/live/evm/tests/support/contract_workflow.rs). The
[generic transaction integration test](crates/live/evm/tests/generic_transaction_runtime.rs)
also exercises framework composition with distinct typed contexts. The [managed task](nixfied.nix)
and [build guide](docs/build-and-verification.md) define the actual service and coverage guarantees.

Existing owners must be reused:

- [Program authoring](crates/kernel/program/src/authoring.rs) provides `Operation`,
  `OperationExpansion`, and `expand_program`. Program v8 is a checked linear sequence, with exact
  initial input commitment, handler selection, allowances, checkpoints, and typed root maps.
- [Runtime](crates/kernel/runtime/src/lib.rs) provides `start`, `resume`, and `read`, exact assembly
  association, continuation transitions, bounded recovery, and Effect barriers. It has no background
  scheduler or caller deadline policy. It validates selected current records without replaying all
  history; Store owns mechanical admission/latest/optional-probe load and atomic append.
- [EVM transaction components](crates/domains/evm/src/transaction/stages.rs) supply checked plans,
  cumulative facts, typed context replacement, and `EvmTransaction<C, R>`. `CallCreatedAt` and
  `ObserveAt` provide specific checked connections. The
  [registration helper](crates/live/evm/src/assembly.rs) installs a transaction family's exact States.
- [Runtime reports](crates/kernel/runtime/src/report.rs) retain failure/root information and stopped
  observations. [Object decoding](crates/kernel/values/src/object.rs) already checks exact contracts.
  The fixture's `root_failure` uses it; success and wallet paths still decode JSON directly.
- [The RPC provider](crates/live/evm/src/json_rpc.rs) already owns
  `fund_development_sender`, sharing bounded transport and causal diagnostics. It supplies one
  unlocked-account submission, not replay-safe funding or funded readiness.
- [Keystore](crates/keystore/src/lib.rs) already owns checked import and signer-handle custody.
  [Application composition](crates/app/src/lib.rs) serves Portfolio products; it does not currently
  provide a general configurable EVM transaction product.

Runtime already implements intrinsic classification, handler selection, retry/checkpoint restart,
canonical failure reports, and `RecoveryStopped`. Failure-routing States and failure regions are
superseded; the current fixture uses `MapFailure` and typed conversions. The missing support must
not be described as a missing recovery engine or a missing generic typed decoder.

## Desired public experience

### 1. Select an existing operation

A caller selects a maintained operation, supplies checked input/options, binds explicit capabilities,
and obtains access to execution and typed results. The caller need not know the selected operation's
internal State list, intermediate contexts, ordinary failure maps, or codec registrations.

Acceptance example: select one supported EVM transaction operation with its checked plan, then
inspect its retained transaction facts. A product may also expose a maintained deployment-and-call
workflow. Selecting that product must not require reauthoring its implementation.

The existing Portfolio entry points remain useful evidence for this path. A new EVM product must
explicitly retain its development-only settlement scope rather than appear in shipping composition
as generally safe production-chain submission.

### 2. Compose existing Operations or States

The caller can declare a sequence and its checked connections, preserve caller-owned context,
and choose supported recovery and root-failure semantics. Authoring a reusable Operation is a valid
way to express that composition. Reusable operations and individual States must remain composable;
a fixed catalogue of complete workflows cannot satisfy this path alone.

Acceptance example, expressed as requirements rather than proposed Rust syntax:

```text
input: contract artifact, static setter argument 42, checked transaction options
sequence:
  deploy the contract
  call configure(42) on that deployment's successful address
  observe value() at the configuration transaction's target and receipt anchor
result: retained execution evidence and a checked decoded value
assertion owned by test: decoded value equals independently chosen 42
```

Changing the sequence or reusing an operation should change the author's semantic declaration,
not require a second manually synchronized list of internal States and maps. Exact executable
requirements still exist and must be checked. The composition boundary must own their association
without making Program or domain crates depend on Runtime or live IO.

An author may deliberately supply a business-specific failure map or report type. The API should
provide supported ordinary propagation without forcing a custom map family merely to sequence
existing behavior. Custom semantics remain explicit; no generic catch-all failure bag should erase
original contracts or causes.

A second acceptance scenario must compose a different context/sequence, including an ordinary call
to an existing address. This prevents solving only the Solidity fixture. A runtime-configurable
step list needs a separately specified bounded representation: current generic context replacement
changes exact Rust types at each stage and cannot simply be concealed behind a fluent builder.

### 3. Implement a new State

An author declares checked input, output, and failure contracts and implements the deterministic
behavior appropriate to Pure, Read, or Effect execution. A new Read/Effect uses an explicit capability;
its adapter owns external IO and, for mutation, its duplicate-entry authority.

Acceptance example: define one small new deterministic transformation, compose it with an existing
operation, and execute it through public framework APIs. Test its useful output and material
rejection/failure behavior. The author should not implement journaling, serialization infrastructure,
a scheduler, a second recovery engine, or unrelated registration boilerplate.

This is a framework extension test. A custom State is legitimate here because extension is what
is being proved. It does not justify rebuilding a parallel product model in Application tests.

## Shared preparation, execution, and result requirements

All three paths converge on the existing checked Program and Runtime:

- Preparation validates the initial input and statically available sequence contracts, selected
  policy, exact executable requirements, and declared capability bindings before admission or
  provider entry. Observation-derived inputs and changing live facts still require validation at
  their existing Runtime/adapter boundaries. Binding live dependencies is distinct from executing
  workflow Effects. Missing support must fail explicitly.
- Caller code retains control over deployment and capabilities, including injected handles for
  library use. Managed service provisioning remains test infrastructure. Reusable composition
  owns normal construction/lifecycle for the paths it advertises; it must not require ambient IO
  in domain code or force library users through CLI/Application products.
- A maintained execution driver may provide polling, deadlines, admission recovery, and supported
  reconnection around Runtime. Direct `start`, `resume`, and `read` remain valid for callers owning
  progression and tests that deliberately exercise it. Repeated consumer copies of an ordinary
  driver are the gap, not every explicit call to `resume`.
- Admitted workflow/recovery semantics and caller attempt policy are separate. Changing a deadline
  or reconnecting must not rewrite Program, initial input, retained command, fees, or failure facts.
  Preserve one explicit RunId and exact recovery identity across ambiguous acknowledgements.
- Typed results reuse `Object::decode` and Runtime's retained reports. Product schemas may expose
  selected names/order, public plans, facts, decoded outputs, and supported checks. Custom business
  reports remain author-owned; independent expected values remain test-owned.
- Preserve durable success, durable failure, and a stopped attempt as distinct observable outcomes.
  A stopped Read can have an operational failure report without a mapped domain root. A stopped
  pending Effect retains command authority through `RecoveryStopped`. Store/internal failures do
  not become durable operational originals, and deadlines do not manufacture terminal records.
- Preserve complete available causes through conversions, under the diagnostic trust contract in
  [design](docs/design.md) and [code quality](docs/code-quality.md). Classification and public rendering
  are projections. Never claim a failed append audited itself or an interrupted attempt was recorded.
- Keep exact schemas, content identities, immutable append-only history, capacity limits, and secret
  exclusion. Reports must retain their declared facts without duplicated context prefixes or an
  implicit Journal lookup; overflow must preserve the acknowledged head and pending authority.

The target is fewer caller obligations and fewer independent implementations. Wrapping the same
fixture helpers behind new names, adding a second lowering pass, or introducing a registry/scheduler
alongside the existing owners does not meet this goal. Exact public APIs and registration mechanics
must be settled before implementation, with deletions identified alongside additions.

## Reassessing the current harness

| Current code or responsibility | Assessment and intended treatment |
| --- | --- |
| `EffectFixtureOperation`, contexts, recipe selection | Legitimate composition when testing framework authoring; a product selection test should consume a maintained operation instead. Simplify obligations rather than ban custom Operations. |
| `register_fixture_states` beside `EffectFixtureOperation::expand` | Duplicated knowledge of selected implementation requirements; remove independent maintenance through the reviewed composition design. |
| ABI helpers and `DecodeValue` | Reusable standard ABI support is a separate product gap. Fixture/business-specific decoding may be legitimate extension; callers should not duplicate codecs already supplied by the selected component. |
| `MapFailure`, `TryFrom`, `FixtureReport`, `FixtureFailure` | Separate intentional product semantics from mechanical propagation and decoding. Retain extension points and causal facts; delete superseded fixture machinery only with equivalent supported behavior. |
| `runtime(...)` | Reusable dependency/assembly construction is mixed with injected reservation faults. Separate maintained composition from deliberate fault setup. |
| `drive_to_success`, resume-before-start handling | Missing reusable ordinary execution policy; deliberate one-step progress/recovery tests still call Runtime directly. |
| `terminal_value`, wallet JSON decoding | Use existing exact-contract decoding and reusable result delivery rather than manual bytes-to-report paths. |
| `generated_signer`, `fund_sender`, compiler artifacts, services | Setup is legitimate. Reusable wallet creation and funding remain separate capability gaps; their presence alone does not establish a framework authoring defect. |
| `ReservationAcknowledgementFault`, SQL/epoch/closed-owner scenarios | Focused integration coverage, with explicit infrastructure and independent boundary assertions. |
| `external_wallet_transfer`, exact heads/nonces, retained-wire checks | Preserve as tests of external activity and durable authority; do not replace them with only happy-path final reports. |
| Expected getter value and business assertions | Independent test oracle. Do not move expected answers into production to make the test smaller. |

## Test organization and acceptance

Tests should make the public use or boundary they prove apparent. This is a division of coverage
responsibility, not a requirement for a new test framework, directory hierarchy, or scenario DSL.

| Test purpose | Required evidence |
| --- | --- |
| Existing-operation selection | Invoke supported production behavior through its public entry point; supply inputs/options and inspect typed results without rebuilding its internal model. |
| Framework composition | Compose reusable Operations and individual States through public authoring APIs; prove useful results, context retention, rejected connections, and deliberate policy behavior. |
| State extension | Implement minimal new semantics using public traits and consume it in a real composition; prove success and relevant failures without duplicating engine tests. |
| Product/transport use case | Use Application and production domain types; preserve independent schema, mapping, rendering, and recovery-identity assertions at the consuming boundary. |
| Fault/recovery integration | Inject faults and assert exact authority/history behavior, including intermediate states when those are the guarantee under test. |

Before moving or deleting existing coverage, map each retained guarantee to its named replacement
scenario and managed task. Preserve reservation acknowledgement loss, prepared-wire recovery with a
rejecting signer, cancellation, ambiguous appends at transaction Journal boundaries, external nonce
advancement, unchanged cold terminal history/output/nonce, SQL cause preservation through cold
Application observation, retained-epoch rejection without append, and closed-owner signing failure.
Capacity/report rejection remains covered by its current owners, using small explicit limits where
appropriate. Do not restore removed redundant capacity matrices.

The managed E2E rebuilds Runtime and IO handles while retaining the same keystore owner; it does not
prove host-process key recovery. Its terminal checks establish unchanged history/output/nonce, not
absence of provider calls. Preserve these limits when describing the reorganized tests.

## Related capabilities retained as separate work

These goals remain relevant, but they do not gate agreeing or improving the three framework paths:

- **Wallet creation:** a reviewed ephemeral-wallet convenience may reuse checked generation/import,
  zeroization, purpose-bound handles, and owner shutdown. No mnemonic or persistent-custody format
  is implied. Secret values never belong in persisted workflow configuration.
- **Development funding:** `DevNodeFundWallet` remains a desired reusable Effect. Its protocol must
  establish readiness and convergent duplicate entry after acknowledgement loss; the existing
  submission hash is insufficient. Reuse bounded RPC transport. A general faucet authority is not
  presumed necessary, and funding must not be disguised as a Read.
- **ABI configuration:** bounded ABI/function or selector selection and static typed inputs require
  a documented supported type catalogue, overload/argument checks, and production codecs. A fluent
  Rust authoring API alone does not provide runtime-configurable arbitrary sequences.
- **Fee discovery:** retained RPC Read evidence can feed deterministic fee rules and bounds; fixed
  fees remain a supported choice. Once prepare is acknowledged, reconciliation reuses the same
  command and fees. New discovery cannot replace retained authority.
- **General output references:** arbitrary prior-output-to-argument wiring is deferred. Existing
  creation/receipt connections remain useful. A future design must check types, dependency order,
  availability after failure, and identity without an untyped context bag.
- **Recovery/finality:** configured product orchestration, reconnection, and process/key lifetime
  remain bounded by existing authority, nonce, and settlement contracts. The transaction adapter's
  pinned non-reorging development policy remains excluded from shipping Application composition;
  general-chain finality requires a separately reviewed capability identity.

[Known gaps](docs/known-gaps.md) tracks these capabilities and their current limitations.

## Decision sequence and complete cutovers

1. Specify reviewable public examples for all three paths, using current primitives as the baseline.
   Inventory caller declarations, repeated assembly work, and result handling. Select exact API and
   registration ownership only after the composition and extension examples work independently of
   a fixed EVM product. Resolve the material design uncertainties below before implementing them.
2. Implement one coherent authoring/association improvement with consuming tests and documentation.
   Preserve inward crate dependencies. Cut over affected callers and delete superseded registration
   or mapping machinery in the same logical commit; do not leave an older path under a wrapper.
3. Implement the reviewed reusable execution/result surface with exact identity, stopped-outcome,
   cancellation, and cold-result tests. Replace ordinary copied drivers/codecs in the same cutover,
   retaining direct Runtime use in boundary tests. Combine with step 2 if their APIs are inseparable.
4. Reorganize consumer and fault scenarios with an explicit coverage map and managed-task selection.
   Delete superseded fixture support only after its semantic and boundary guarantees have owners.
   Product-specific ABI, wallet, funding, and fee additions can then proceed as separate capabilities.

Each implementation commit must leave one coherent current design and report simplified/removed
machinery, necessary added complexity, and production-code LOC change. Changes to responsibility,
Program, Runtime, or persistence contracts must update architecture/design and affected tests in
the same cutover. No migration, compatibility wrapper, or second execution model is implied.

Completion means all three public paths are demonstrated and documented, ordinary consumers no
longer duplicate supported machinery, framework authors retain expressive typed composition and
extension, and existing durability/error guarantees retain independent coverage. A smaller EVM test
alone is not sufficient evidence.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation before implementation |
| --- | --- | --- | --- |
| Composition can couple semantic selection to exact executable requirements without duplicate lists. | Program/domain crates cannot depend on Runtime; generic context replacement changes exact State ABIs. | A convenience layer could add another registry/lowering path or break crate boundaries. | Architect one concrete ownership design using two contexts, nested Operations, individual States, maps, and capability injection; enumerate the deleted machinery. |
| Supported ordinary failure propagation can be simpler while preserving custom maps. | Products choose different root schemas and fact retention. | A generic shortcut could erase causes, lose facts, or hide meaningful product policy. | Specify ordinary and custom-root examples, operational failure without a root, report overflow, and cold decoding. |
| A bounded configured EVM surface can share framework components. | Runtime-selected step counts/order and ABI types do not map automatically to current monomorphized contexts. | The product could require a parallel execution model or expose an unbounded expression system. | Specify supported step/result representation and ABI rejection cases, then compare two distinct workflows. Keep this separate from Rust composition acceptance. |
| One reusable driver can express supported attempt policy honestly. | Admission ambiguity, pending Effects, cancellation, deadlines, reconnection, and key lifetime interact. | Retry could change authority or stopped attempts could be reported as durable failures. | Define the outcome/action matrix and test exact start/resume identity and each affected interruption boundary. |
| Development funding can use a small convergent Effect protocol. | Unlocked-account submission lacks acknowledgement-loss duplicate suppression and readiness evidence. | A wrapper could fund twice or return before funds are usable. | Review the node protocol and test duplicate entry, readiness, and lost acknowledgement before implementing the separate funding capability. |

## Verification of this RFC revision

This is a documentation-only cutover: README positioning, this RFC, related gap/test guidance, and
removal of the superseded problem statement. Review local links/anchors, cited public symbols,
consistency with authoritative contracts, absence of stale references, and `git diff --check`.
Production-code LOC change is zero. No Rust tests, managed E2E, or CI run is selected by the
[build guide](docs/build-and-verification.md) for this revision. Subsequent code/task changes must
select their focused checks and final CI according to that guide.
