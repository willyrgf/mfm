# RFC: reshaping MFM's public interfaces and tests

## Status and scope

The three public usage paths below are the agreed framing for MFM as a platform and framework.
One representative E2E scenario per path is the agreed replacement for the current E2E organization.
Each scenario can grow named case runs for recovery, durability, and error handling.
This RFC proposes requirements and acceptance scenarios for improving those paths. The linked
architect designs now specify concrete target journeys and proposed caller APIs. They start from
the desired public experience; current API limitations are evidence for production changes, not
constraints on the tests. Exact signatures and the implementation proofs identified in those designs
remain to be settled. No proposed API is available merely because it appears here.

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

The selected E2E journey is a Portfolio snapshot through both CLI and REST, with independently
checked balances. It isolates selection of maintained behavior without requiring authoring or
transaction custody. A library caller can similarly select a supported EVM transaction operation;
a future product may expose a maintained deployment-and-call workflow. Selecting such behavior
must not require reauthoring its implementation.

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
input: checked production config with initial value 42, increment 42, artifact and transaction options
maintained ContractDeploymentLifecycle operation:
  Deploy -> Configure -> Observe -> Validate -> Report, yielding 42
recomposition through the existing OperationExpansion DSL:
  reuse Deploy
  insert the existing checked-add Pure State, producing argument 84
  reuse Configure, Observe, Validate and Report
result: retained execution evidence and checked value 84
assertion owned by test: result differs from the unchanged operation's independently expected 42
```

For this E2E, production owns the context, config, component types and ordinary failure maps.
The caller obtains production step selections directly and declares a new operation through the
same authoring constructor used by the maintained lifecycle; no original instance or separate
transformation constructor is needed. It defines no structs, type aliases or new States. Recomposition validates a new immutable Program,
not an edit of lowered or admitted history. General framework extension can still define new types.
Deploy and Configure are business Effect States, Observe is a Read State, and Validate/Report are
Pure States. Their typed selections carry capability setup and trigger the existing capability-owned
injection through the current DSL. The lifecycle is the reusable Operation grouping those steps.
The root authoring entry should accept either an Operation or a checked State selection; this removes
unnecessary one-State wrappers without introducing `State::expand` or another injection owner.

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
- One public Runtime execution surface provides bounded ordinary progression, with a builder for
  explicit dependencies and checked options. It exposes no separate public driver object. Direct
  `start`, `resume`, and `read` remain valid for callers owning progression and boundary tests.
  Repeated consumer progress loops are the gap, not every explicit call to `resume`.
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

## Three public-usage E2Es

The target is one representative E2E scenario for each public usage path. Together they replace the
current E2E organization and demonstrate the platform and framework experiences. This is a scenario
boundary, not a fixed count of Rust functions, binaries, or managed tasks. Each scenario may contain
multiple named, independently diagnosable case runs. No new test framework or scenario DSL is implied.

| E2E scenario | Public surface and caller-owned work | Baseline evidence |
| --- | --- | --- |
| 1. Select an existing operation through CLI/REST | Select supported production behavior and supply its inputs/options through the client transports. Use the shared Application surface and production domain types. | Execute the same representative use case through both CLI and REST, retaining transport-specific rendering and recovery-identity assertions. Inspect meaningful results without reconstructing the operation's internals. |
| 2. Compose existing Operations and States | Use public Rust authoring APIs to declare order, valid connections, caller context, and intentional policy; bind explicit execution capabilities. | Recompose a maintained lifecycle Operation from its existing State selections, insert an existing State, and execute through the public framework with checked results. Do not implement a new State to make the composition work. |
| 3. Implement a new State and compose it | Define one small State's new semantics and typed contracts through public traits, then combine it with existing Operations/States. | Execute that composition through the public framework and assert the new behavior and integration result. The new State must express actual semantics, not hide missing composition machinery. |

The first scenario covers two transports over one Application surface; passing only CLI or only
REST is insufficient. Its baseline can select a shipping Portfolio operation. It does not require
adding EVM transaction submission to shipping composition. The second and third are library-facing
framework tests and must remain usable without going through the CLI or an Application product.
Any live transaction cases retain the explicit development settlement scope.

A test-owned Operation is legitimate in scenarios 2 and 3 because composition is their public use.
Their surrounding execution, association, and result handling should consume maintained framework
support. Scenario 2 must expose composition gaps rather than work around them with a custom glue
State. Scenario 3 must demonstrate extension rather than duplicate an existing component.

### Case runs and coverage growth

Start by designing a clear baseline journey for each scenario. Additional named cases can then
exercise rejection, failure, cancellation, stopped/resume behavior, acknowledgement loss, cold
reconstruction, or other relevant boundaries inside the same public-usage scenario. These are
specific runs with explicit setup, an injected condition where needed, an independently stated
expectation, and a clear outcome; they need not become one long stateful test.

The three scenarios cover the public usage paths. Their count alone does not establish every
recovery, durability, or error-handling guarantee. Future cases can strengthen that coverage, while
existing guarantees must retain an owner during replacement. Fault cases may use deliberate
infrastructure and intermediate history/authority assertions when those are the behavior being
proved. Keep fault setup distinguishable from the public journey, and keep expected answers in
the test. Do not require faults to become production operation options.

Focused unit, compile-fail, and integration tests remain useful for guarantees most directly tested
at their owning boundary. A case does not have to move out of an E2E merely because it exercises a
fault, nor does every low-level boundary need repetition in all three E2Es. Existing real-service
requirements remain where the guarantee depends on PostgreSQL or the EVM node.

### Replacing the current E2Es

The current [client execution E2E](bin/rest-api/tests/client_execution_e2e.rs) and
[contract Effect E2E](crates/live/evm/tests/evm_contract_effect_e2e.rs), together with their support
code and managed task selection, are the starting inventory. Replace them through the three
scenarios rather than keeping the old E2Es as a parallel acceptance suite. Reuse or reshape existing
files where appropriate; their names do not determine the target coverage.

Before deleting an old scenario or assertion, record its guarantee, the named replacement case or
focused test that owns it, and the managed task that runs it. Future planned cases do not justify
removing currently exercised guarantees. This mapping must include:

- Client transport execution and cold resume after config deletion, exact snapshot observation,
  independent CLI execution, retained provider causes through cold CLI/REST observations, and
  enrichment/publication/dependent-start recovery with deleted source revisions.
- Reservation acknowledgement loss, prepared-wire recovery with a rejecting signer, cancellation,
  ambiguous appends at transaction Journal boundaries, external nonce advancement, unchanged cold
  terminal history/output/nonce, SQL cause preservation through cold Application observation,
  retained-epoch rejection without append, and closed-owner signing failure.
- Existing capacity/report rejection at its current focused owners, using small explicit limits
  where appropriate. Do not restore removed redundant capacity matrices.

The [build guide](docs/build-and-verification.md) records the current managed guarantees. Its task
coverage and `nixfied.nix` selection must change with the executable test cutover, not in advance of
it. This RFC neither deletes tests nor claims that the replacements already exist.

The managed Effect E2E rebuilds Runtime and IO handles while retaining the same keystore owner; it
does not prove host-process key recovery. Its terminal checks establish unchanged history/output/
nonce, not absence of provider calls. Preserve these limits in replacement cases.

### Concrete architect designs

One architect designed each scenario from its objective. These documents specify the intended
public calls, independent oracles, named case runs, production responsibilities, and cutover scope:

| Scenario | Target journey | Detailed design |
| --- | --- | --- |
| Selection | Select a Portfolio snapshot through both real transports; execute independently and verify fixture balances, then cold-observe through the other client. | [CLI/REST selection](docs/e2e-selection-design.md) |
| Composition | Rebuild ContractDeploymentLifecycle from its Effect/Read/Pure step selections through the current DSL, inserting a Pure addition State so configured `42` produces reported `84`. | [Compose existing components](docs/e2e-composition-design.md) |
| Extension | Introduce one meaningful deterministic policy State and compose it with maintained components; prove typed success and rejection. | [Implement and compose a State](docs/e2e-extension-design.md) |

These are target test designs, not descriptions of tests already implemented. Their caller sketches
are reviewable API proposals. Production must supply the machinery they require; test helpers must
not hide missing composition, registration, execution, or decoding support. Existing implementation
shape may change where necessary, with authoritative contracts updated in the same code cutover.

The common direction reuses the current `OperationExpansion` DSL, explicit capabilities, and one
public Runtime construction/execution surface. No parallel sequence DSL or public driver is selected. Operation selection must carry exact executable
requirements through the same authoring/lowering path. Ordinary composed domain failures retain
typed payloads; custom product maps remain intentional authoring. CLI/REST Application execution
and library callers share Runtime execution rather than acquire separate progress loops. Wait
budgets govern attempts, not admitted recovery authority.

Runtime execution follows one shared action contract; its exact types and transport spellings
must be implemented together:

| Observed condition | Runtime execution behavior |
| --- | --- |
| Runnable, awaiting recovery, or awaiting interpretation | Continue eligible Runtime progression within the invocation budget; Runtime authorizes each transition. |
| Pending Effect without a stopped recovery decision | Poll the same retained command within the budget; never prepare a replacement. |
| Durable success or failure | Return the existing terminal result with checked output or failure access. |
| `RecoveryStopped` | Return the unresolved observation; do not bypass the selected handler by automatically resuming it. |
| Store/invocation failure or ambiguous append acknowledgement | Return the preserved causal failure and exact available recovery identity; no invisible fresh admission or assumed commit. |
| Invocation deadline | Stop driving and report the exact RunId and last qualified observation if available. Do not claim that historical observation is the current head or invent a terminal failure. |
| Cancellation or lost client/process | Preserve Runtime cancellation guarantees; a disconnected caller may receive no response. Explicit recovery uses the retained identity, not a promised cancellation record. |

Before implementing, reconcile exact signatures and validate the proposed registration mechanism
with nested Operations, individual States, capability injection, custom maps/handlers, and distinct
context types. Resolve the shared execution outcome/action contract for deadline, cancellation,
ambiguous append, durable failure, and stopped pending Effects. These feasibility checks must not
relax the three public objectives to fit current APIs.

Each design distinguishes baseline/new-behavior cases, coverage required when replacing current
tests, and optional future cases. Complete the assertion-to-case and managed-task mapping before
removing old tests. Keep desired behavior and independent expected results visible in the test.

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

1. Validate the linked target designs and settle their shared public signatures and ownership
   mechanisms. Use current primitives as implementation evidence, not as limits on the caller
   experience. Map current guarantees to replacement cases or focused tests. Prove composition and
   extension independently of a fixed EVM product, and resolve the relevant material uncertainties
   before implementing the production cutover.
2. Implement one coherent authoring/association improvement with consuming tests and documentation.
   Preserve inward crate dependencies. Cut over affected callers and delete superseded registration
   or mapping machinery in the same logical commit; do not leave an older path under a wrapper.
3. Implement the reviewed Runtime execution/result surface with exact identity, stopped-outcome,
   cancellation, and cold-result tests. Replace ordinary copied drivers/codecs in the same cutover,
   retaining direct Runtime use in boundary tests. Combine with step 2 if their APIs are inseparable.
4. Replace the current E2E organization with the three public-usage scenarios and their required
   case runs, using the coverage map and updated managed-task selection. Retain focused boundary
   tests where appropriate. Delete superseded E2Es/support in the same cutover once their guarantees
   have executable owners. Extend each scenario with additional cases as needed; product-specific
   ABI, wallet, funding, and fee additions remain separate capabilities.

Each implementation commit must leave one coherent current design and report simplified/removed
machinery, necessary added complexity, and production-code LOC change. Changes to responsibility,
Program, Runtime, or persistence contracts must update architecture/design and affected tests in
the same cutover. No migration, compatibility wrapper, or second execution model is implied.

Completion means the three public-usage E2Es replace the prior organization and demonstrate all
three paths, with both CLI and REST covered by selection. Ordinary consumers no
longer duplicate supported machinery, framework authors retain expressive typed composition and
extension, and existing durability/error guarantees retain independent coverage. A smaller EVM test
alone is not sufficient evidence.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation before implementation |
| --- | --- | --- | --- |
| Current E2E guarantees can be assigned to the three scenarios or focused tests without loss. | Concrete target stories are specified, but the exhaustive assertion-to-case inventory is incomplete. | Premature replacement could drop unique behavior or create oversized scenarios that obscure failures. | Complete the coverage map, name required case runs and managed tasks, and distinguish preserved coverage from optional future additions before deleting tests. |
| Composition can couple semantic selection to exact executable requirements without duplicate lists. | Program/domain crates cannot depend on Runtime; generic context replacement changes exact State ABIs. | A convenience layer could add another registry/lowering path or break crate boundaries. | Validate the proposed Program-owned authoring sink with two contexts, nested Operations, individual States, maps, and capability injection; enumerate the deleted machinery. |
| Supported ordinary failure propagation can be simpler while preserving custom maps. | Products choose different root schemas and fact retention. | A generic shortcut could erase causes, lose facts, or hide meaningful product policy. | Specify ordinary and custom-root examples, operational failure without a root, report overflow, and cold decoding. |
| A bounded configured EVM surface can share framework components. | Runtime-selected step counts/order and ABI types do not map automatically to current monomorphized contexts. | The product could require a parallel execution model or expose an unbounded expression system. | Specify supported step/result representation and ABI rejection cases, then compare two distinct workflows. Keep this separate from Rust composition acceptance. |
| One Runtime execution surface can express supported attempt policy honestly. | Admission ambiguity, pending Effects, cancellation, deadlines, reconnection, and key lifetime interact. | Retry could change authority or stopped attempts could be reported as durable failures. | Define the outcome/action matrix and test exact start/resume identity and each affected interruption boundary. |
| Development funding can use a small convergent Effect protocol. | Unlocked-account submission lacks acknowledgement-loss duplicate suppression and readiness evidence. | A wrapper could fund twice or return before funds are usable. | Review the node protocol and test duplicate entry, readiness, and lost acknowledgement before implementing the separate funding capability. |

## Verification of this RFC revision

This revision links concrete architect designs for the three public-usage E2Es; it changes no
executable tests, public APIs, or task selection. Review local links/anchors, cited public symbols,
consistency with authoritative contracts, absence of stale references, and `git diff --check`.
Production-code LOC change is zero. No Rust tests, managed E2E, or CI run is selected by the
[build guide](docs/build-and-verification.md) for this revision. Subsequent code/task changes must
select their focused checks and final CI according to that guide.
