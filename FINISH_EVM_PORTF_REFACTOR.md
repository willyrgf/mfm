# Finish the EVM and Portfolio single-trust-boundary refactor

Status: implementation plan. This document does not change the normative RFC or its original
implementation plan. It records the corrective work required before their completion can be
claimed.

Baseline: `bc103f891a9576bba98b18adc437ad6f201fc9e0`.

## Outcome

Yes: EVM collection, EVM submission, and Portfolio should have been cut over to the refactored
Program/Store/Runtime core. The current tree has most of the new callback-free data model, but it
does not contain production EVM or Portfolio `State` implementations or a usable live assembly.
App still authors both Programs, invents physical bindings from implementation-name strings, and
can admit actionable runs with no Runtime. CLI and REST construct exactly that empty-runtime mode.

The finish is one deletion-led vertical cutover:

```text
transport selector/request
        |
        v
domain planner + trusted typed config + secret-free exact bindings
        |
        +----> one domain-owned C0
        `----> one final input-shaped Program
                         |
                         v
one catalog-wide RuntimeAssembly + one Store mutation port
                         |
                         v
stable domain State implementation -> exact bound adapter -> provider
```

There will be no second workflow model, per-input Runtime, runtime loop, Program cache, optional
execution path, or compatibility fallback.

## Material uncertainties

Implementation must not begin until the following contract questions are frozen in the first
commit.

1. **Canonical public EVM/Portfolio result baseline.**

   - Choice/assumption: “preserve existing public canonical bytes” refers to the last
     pre-cutover tree, `b223ae17e07844d59df05e61ea253b1dedfb90c6`, rather than the simplified
     result types introduced by `2cd39f945dcf14713911cedc89cb2ba9e2c4646d`.
   - Why uncertain: the current `PortfolioSnapshotOutput` is only collection totals, the current
     `EvmBalanceCollectionResult` drops the documented ordinal/correlation metadata, and the
     current `EvmSubmissionOutput` replaced the earlier `execution_disposition` wire. The RFC and
     implementation plan explicitly require the prior EVM collection and Portfolio public wire
     semantics, but no final golden was retained in the cutover tree.
   - Consequence if wrong: the implementation could faithfully execute the new core while
     permanently blessing an already-regressed public contract, or could restore a large obsolete
     model that the product no longer wants.
   - Resolution: extract canonical success/failure goldens and a field-ownership table from
     `b223ae17...` in commit 1. Restore only the value/projection semantics required by those
     goldens; do not restore the former workflow, fan-out, wallet lifecycle, or certification
     machinery. If a different product contract is intended, update `docs/design.md`, this plan,
     and the goldens deliberately before code work continues.

2. **Meaning of EVM submission inclusion.**

   - Choice/assumption: a broadcast transaction hash is not proof of inclusion. The recommended
     target is the historical public `execution_disposition` (`succeeded | reverted`), supported
     by explicit receipt/finality Reads after broadcast.
   - Why uncertain: the current output has `included: bool`, but neither its source nor its trust
     semantics is specified. Current `BroadcastEvidence` proves only an authenticated provider
     response to broadcast.
   - Consequence if wrong: setting `included` during broadcast fabricates chain state; restoring
     disposition without receipt/finality States would make the same false claim under another
     name.
   - Resolution: the commit-1 contract table must identify the exact provider evidence for every
     public field. If the product intentionally wants only provider acceptance, rename and freeze
     that weaker result. Otherwise include bounded receipt/finality Read States and their exact
     failure policy in the final Program before implementation begins.

3. **Standalone CLI/REST disposition.**

   - Choice/assumption: because the RFC excludes production provider topology, endpoint
     credentials, key custody, and binary assembly, the standalone demo binaries must stop exposing
     actionable admit/drive surfaces. App remains the injectable production library boundary.
   - Why uncertain: keeping runnable admit/drive commands requires a concrete trusted deployment
     composition that is explicitly outside this task, while deleting transport commands is a
     visible product change.
   - Consequence if wrong: retaining `Vec::new()` keeps admitting permanently undriveable runs;
     installing a fake/null provider creates a second false production path.
   - Resolution: confirm removal of the standalone actionable commands/routes in commit 1. If a
     deployable binary is required, define its real provider/topology/key-custody scope separately;
     it must not be smuggled into this refactor.

All other architectural choices are settled by the RFC, repository architecture, and the
architect review summarized below.

## Current-tree findings

The prior completion record is not sufficient evidence for EVM/Portfolio completion:

- `crates/domains/evm` and `crates/domains/portfolio` define values but contain no production
  `impl State` or `impl FailureValue`; the only implementations are Runtime tests.
- `crates/app/src/lib.rs` owns `evm_submission_program`, `portfolio_program`, address allocation,
  declaration helpers, occurrence-shaped planning structs, and ordinal-formatted implementation
  identities. This violates the documented operation/domain ownership boundary.
- App creates adapter and physical-target identities by hashing implementation-name strings. Those
  identities cannot prove equality with an actual `EvmPhysicalTarget` or live binding.
- `EvmNativeBalanceInput` and `EvmTokenBalanceInput` carry only a selected source. Match therefore
  drops the cumulative EVM context and opaque Portfolio continuation instead of handing the exact
  complete context to the selected arm.
- `EvmBalanceContext` has stage flags and optional fields but lacks the checked chain, initial
  anchor, decimals, and observed balance required by the RFC's cumulative ABI.
- Runtime owns one exact Program and requires one registration per declaration. Input-unrolled
  Portfolio Programs therefore force occurrence-specific implementation identities instead of one
  reusable implementation per semantic State.
- Runtime Pure and accepted-Access APIs borrow inputs. They cannot move a non-`Clone` opaque caller
  continuation into the successor value as required by the RFC.
- App cold resume rebuilds a Program from retained `C0` using current code. The RFC requires resume
  to consume the persisted final control form without rerunning planning or expansion.
- App accepts `Vec<Runtime>`, maintains an entry-point Runtime map, falls back to direct Store
  admission, and returns an error when such a run becomes actionable. Both binaries pass
  `Vec::new()`.
- The live EVM adapter is not registered by production composition, duplicates fields already
  present in `BindingDescriptor`, and its broadcast intent omits transaction material needed to
  derive the provider request solely from the committed intent.
- `ReadWalletNonceStatus` is classified as a Read even though the available wallet authority must
  durably reserve a nonce. The current context also requires a candidate before its advertised Pure
  derivation State.
- Current failure-route States claim an output contract they cannot honestly construct. Store
  already has a typed `Failed` terminal action; Program validation prevents ordinary fail-fast use
  of it.

## Architect consensus

Focused Runtime, EVM, Portfolio, adapter, verification, and simplicity reviews reached one target:

- make Runtime assembly catalog-wide and Program-independent;
- persist the exact normalized Program at admission and load it on cold paths;
- move callback-free State contracts to the domain-facing Program crate;
- consume typed inputs at the semantic handoff instead of cloning them;
- register each stable semantic State implementation once and reuse it across occurrences;
- keep one live Runtime registry, with State implementations keyed by semantic implementation ref
  and adapters keyed by exact binding ref;
- move EVM/Portfolio planning, authoring, transitions, evidence interpretation, and consolidation
  out of App and into their domain crates;
- use actual trusted `BindingDescriptor`s and physical-target identities;
- require App to own one Runtime and delete structural-only execution;
- add no crate, external dependency, scheduler, cache, generic workflow context, second registry,
  or compatibility path.

## Target design

### 1. Persist and recover the exact Program

`ProgramDocument::program_ref()` must use one dedicated Program-document schema identity, not the
root result's schema identity.

Admission adds the canonical `ProgramDocument` as an immutable `mfm.program` object in the same
atomic `RunAdmitted` object closure as `C0`. The existing `RunAdmitted.program_ref` references that
object; no fourth run-record family and no new journal field are needed.

Update closure/reachability rules so the Program object is mandatory and reachable. Missing,
duplicate, corrupt, noncanonical, substituted, wrong-ref, wrong-entry, or foreign-catalog Program
objects fail qualification.

The Store mutation path becomes conceptually:

```text
history.admit(run_id, &program, &c0, configuration, sources)
history.select(run_id) -> SelectedRun owning the exact catalog-qualified Program
```

`select` loads the admission object closure, strictly ingresses the retained document under the
Store's exact catalog, checks `program_ref`, then reduces the prefix. Runtime resume accepts only a
`run_id`; it never accepts or reconstructs a Program. Hot `RunSession`, `CommittedCall`, suspended
owners, and pending conclusions retain the selected Program identity.

Reader, replay, audit, terminality, and portable export consume the same retained document through
callback-free qualification. Delete every API requiring App to supply a reconstructed document.
Portable export includes the Program object exactly once.

### 2. Put callback-free State contracts in Program

Move the existing `State` and `FailureValue` traits from `mfm-runtime` to `mfm-program`, which is
already domain-facing and already owns State declarations and nominal contracts. Update every
consumer in the same cutover; Runtime no longer exports a duplicate or compatibility name.

Derive the implementation content identity from `S::state_id()` with one Program-owned helper.
Registration no longer accepts an unrelated caller-supplied `ContentRef`.

Delete Runtime's unused public `Pure`, `Read<C>`, and `Effect<C>` marker structs. Execution mode
remains callback-free Program/capability data.

Change semantic execution to preserve affine, non-`Clone` contexts:

```text
Pure:   evaluate(S::Input) -> ProposedStateOutcome<S::Output, S::Failure>
Access: prepare(&S::Input) -> Intent
        interpret(S::Input, accepted Evidence)
          -> ProposedStateOutcome<S::Output, S::Failure>
```

The adapter consumes `CommittedCall` and authenticates evidence. Runtime then gives the retained
typed input and accepted evidence to the State's one consuming interpretation callback. Runtime
retains content identity/correlation, not a second copy of the value. Do not add `Clone` bounds or
serialize/redecode the hot handoff.

### 3. Make Runtime assembly catalog-wide

`RuntimeAssembly` owns exactly one `ProgramCatalog`, one private process registry, and one brand. It
does not own a Program or entry-point id.

Use one private registry with two exact indexes, not two authorities:

- semantic State implementation ref -> typed Pure/Access implementation;
- execution-binding ref -> typed adapter invocation for that State/capability.

`register_pure::<S>` and `register_access::<S, C>` register semantics once. Access adapter
registration associates any finite set of exact `BindingDescriptor`s with that semantic State.
Repeated declarations can use the same State implementation ref; their control addresses identify
occurrences, and their binding refs select physical execution.

Assembly construction rejects duplicate/conflicting type, mode, capability, binding, target,
signer, and effect-domain registrations. Because one assembly serves both entry points and multiple
input-shaped Programs, a particular Program may use a subset. Before `RunAdmitted`, Runtime validates
that every declaration in the supplied Program resolves exactly; no callback or provider can run
on failure. Application construction additionally rejects registrations unreachable from the two
selected typed configurations and supplied route descriptors, preventing dormant topology from
becoming a general plugin registry.

The Runtime surface becomes:

```text
RuntimeAssemblyBuilder::new(catalog)
  .register_pure::<S>(implementation)
  .register_access::<S, C>(implementation)
  .register_adapter::<S, C>(binding, invoke)
  .finish()

Runtime::new(assembly, history_port)
Runtime::admission(run_id, program, c0, configuration, source_refs)
Runtime::resume_run(run_id)
```

Do not add `RuntimeFactory`, `BoundProgram`, a Program cache, a per-run Runtime map, or a dynamic
scheduler.

### 4. Make planning trusted and domain-owned

Portfolio transport accepts only a bounded public selector:

```text
PortfolioSnapshotSelector { target, quote }
```

It does not accept collections, configuration, route refs, adapter identities, or a planned C0.
Unknown fields fail.

Trusted composition retains the typed `ResolvedConfiguration<PortfolioConfig>` and a finite
secret-free set of exact EVM binding descriptors produced alongside the live Runtime
registrations. `PortfolioConfig` is the declarative holding plan for the MVP: Portfolio identity,
allowed quotes, and bounded ordered collection/source configuration. Extend it only with fields
required by the frozen public-result contract.

One domain function performs selection, validation, binding resolution, and pure static expansion:

```text
plan_snapshot(selector, &typed_config, exact_bindings)
  -> PortfolioAdmissionPlan { c0, program, source_refs }
```

This is one concrete result product, not a planner trait or registry. Its private construction
ensures `C0`, Program, configuration, and source refs cannot diverge. App only dispatches the entry
point, qualifies the result under the one catalog, derives the run id, and calls Runtime.

`PortfolioSnapshotInput` is the domain-planned C0, not a transport DTO. It contains the selected
target/quote, exact bounded collection demand, and the secret-free route/binding identities needed
for continuation and result validation. Private constructors and hostile decoding enforce:

- nonempty demand;
- at most 64 collections and at most 64 total EVM sources;
- target/config and quote membership;
- dense declaration order;
- unique source identities;
- exact chain/route/binding agreement;
- no secret marker or unbounded string/vector.

EVM submission planning likewise validates the caller request against trusted EVM config and exact
route/signer bindings before producing its C0 and Program. A caller can select only a deployment-
supplied route; App never fabricates one.

### 5. Implement the EVM balance fragment as reusable States

Replace the correlated stage flag, optional current source, stored cursor, and stored remaining
suffix with one data-carrying work sum:

```text
EvmBalanceContext<K> {
  request,
  caller_continuation: K,
  metadata,
  completed,
  work: EvmBalanceWork,
}

EvmBalanceWork =
  CheckChainIdentity { source }
  | ReadInitialAnchor { source, checked_chain_id }
  | SelectAsset { source, checked_chain_id, initial_anchor }
  | ReadNativeBalance { source, checked_chain_id, initial_anchor }
  | ReadTokenDecimals { source, checked_chain_id, initial_anchor }
  | ReadTokenBalance {
      source, checked_chain_id, initial_anchor, token_decimals
    }
  | ConfirmAnchor {
      source, checked_chain_id, initial_anchor,
      source_decimals, raw_balance
    }
  | Complete
```

The next ordinal and remaining suffix are derived from `completed.len()`, the active work variant,
and the admitted request. Do not persist redundant cursor/suffix fields. Every constructor and
decoder proves `completed + active + remaining == admitted demand` in exact declaration order.

Replace all source-only Match values with one closed sum:

```text
EvmBalanceAsset<K> =
  Native(EvmBalanceContext<K>)
  | Token(EvmBalanceContext<K>)
```

Both variants contain the full cumulative context and both arms declare
`EvmBalanceContext<K>` as the exact payload/first-State input contract. Delete
`EvmNativeBalanceInput`, `EvmTokenBalanceInput`, and their source-only `EvmBalanceAsset` shape.

Use one stable State type/implementation id for every semantic stage, reused by all unrolled
sources and collections:

| State | Mode | Responsibility |
| --- | --- | --- |
| `check-chain-identity@1` | Read | authenticate exact chain identity |
| `read-initial-anchor@1` | Read | capture the initial block anchor |
| `select-asset@1` | Pure | construct the complete native/token Match value |
| `read-native-balance@1` | Read | observe native raw units at the selected anchor |
| `read-token-decimals@1` | Read | observe token decimals |
| `read-token-balance@1` | Read | observe token raw units at the selected anchor |
| `confirm-balance-anchor@1` | Read | confirm the anchor, append one source result, advance work |
| `consolidate-balance-collection@1` | Pure | validate and construct the collection result |

Static authoring unrolls those occurrences before admission. There is no Runtime loop. Use a
structured `EvmReadIntent` subject sum instead of concatenated operation/subject strings; evidence
retains and checks the exact intent. `ReadBalance` and `ReadLatestAnchor` remain the small reusable
capability families, but accept only the variants assigned to them.

`EvmBalanceConsolidation<K>` consumes `Complete`, validates exact demand realization, binding,
common anchor, source uniqueness/order, decimals, scaled integer arithmetic, and metadata, then
returns:

```text
EvmBalanceCollectionCompletion<K> {
  caller_continuation: K,
  result: EvmBalanceCollectionResult,
}
```

The result retains the frozen ordinal/correlation metadata exactly once. `K` is moved unchanged;
EVM exposes no Portfolio accessor or erased byte representation.

Use one EVM-owned `EvmBalanceFailure` containing only the reviewed stage, collection correlation,
and redacted code needed by its caller. It must not depend on Portfolio types.

### 6. Implement the Portfolio State chain

Simplify the continuation to one valid-by-representation value:

```text
PortfolioContinuation { input, completed_collections }
```

Derive current ordinal and remaining demand from `completed_collections.len()` and C0. Delete stored
`next_collection` and `remaining_collections` fields.

Use stable reusable Portfolio States:

| State | Input -> output |
| --- | --- |
| initialize | planned C0 -> empty `PortfolioContinuation` |
| enter collection | continuation -> complete `EvmBalanceContext<PortfolioContinuation>` |
| resume collection | EVM completion -> continuation with exactly one appended result |
| map EVM failure | EVM failure -> typed Portfolio failure |
| consolidate | complete continuation -> frozen public output |

Authoring unrolls one enter/EVM-fragment/resume sequence per collection. The next collection has no
incoming edge from any failure path. The mapper is a genuine State that always returns its declared
`PortfolioSnapshotFailure`; it does not manufacture a success output. EVM failures include the
collection ordinal needed for `CollectionFailed { ordinal, code }` but do not interpret Portfolio
semantics.

Final consolidation owns cross-collection binding/order validation, checked integer/decimal
mathematics, and the exact public snapshot/report projection selected in commit 1. No valuation,
config, or routing semantics remain in App.

### 7. Make ordinary typed failures terminal

Program validation must allow a State with a declared failure contract and no `failure_next` to
terminate the run as `RunAction::Failed`, regardless of whether the success path is terminal. The
root result contract continues to govern successful terminal paths only.

Store validates and persists the typed failure exactly as today, then stops reduction. Runtime and
App return the redacted failed status only after the conclusion is durable. Integrity-blocked paths
use the domain's `FailureValue::integrity_blocked()` and the same declared failure rule.

Delete EVM and Portfolio terminal “failure States” whose only purpose is to pretend a failure can
produce the success root contract. Retain an explicit Portfolio mapper only where ownership changes
from `EvmBalanceFailure` to `PortfolioSnapshotFailure`.

### 8. Correct EVM submission instead of wiring its skeleton

The minimal honest nonce/candidate/broadcast prefix is:

```text
ReserveWalletNonce     EvmSubmissionRequest -> EvmNonceReserved
                       Effect<EntryOnce>
DeriveCandidate        EvmNonceReserved -> EvmSubmissionContext
                       Pure
BroadcastTransaction   EvmSubmissionContext -> broadcast successor/failure
                       Effect<EntryOnce>
```

Replace `ReadWalletNonceStatus` with `ReserveWalletNonce`. Reservation is durable mutation even if
the backing row is idempotent. Acknowledgement-unknown parks permanently and never reserves a
second nonce. Do not restore absorbing-Effect machinery.

`EvmNonceReserved` owns request plus nonce. Only `DeriveCandidate` computes the deterministic
candidate. `BroadcastIntent` contains the complete unsigned transaction and public execution
identity: target/chain, data, gas, fees, sender, nonce domain, nonce, idempotency key, candidate id,
and public signer key-instance ref. Adapter signing/provider bytes derive only from that committed
intent.

The exact post-broadcast receipt/finality suffix is frozen by Material uncertainty 2. It must be
explicit Read States if the public result claims inclusion or execution disposition. It cannot be
folded into broadcast response handling or set to a constant.

The wallet storage `completed` flag and `complete()` operation currently have no semantic consumer.
Delete them and their SQL column unless the frozen submission contract assigns them an exact
completion owner.

### 9. Bind real adapter identities without adding topology machinery

`BindingDescriptor` remains the sole persisted execution-binding representation.
`EvmAdapterBinding` retains one validated descriptor, the actual `EvmPhysicalTarget`, public
sender/nonce/signer metadata needed for validation, and the injected provider/nonce/signer handle.
Delete the duplicated state, capability, adapter, execution-binding, target, effect-domain, and
signer-ref fields and its many-argument constructor.

Trusted EVM composition creates live registrations and the corresponding secret-free planning
descriptors together. This is one concrete composition function/product, not a public generic
routing or factory framework. It validates:

- physical target ref from `EvmPhysicalTarget::content_ref()`;
- stable adapter implementation identity;
- exact State and capability contracts;
- absence of signer/effect fields on Reads;
- nonce-reservation effect domain from tenant/sender/nonce domain;
- broadcast effect domain and public signer key-instance identity;
- uniqueness and complete reachability for the selected App configuration.

The only provider-entering path consumes `CommittedCall`. A provider, signer, nonce authority, or
adapter handle alone is inert. Wrong target, State, capability, binding, effect domain, signer, or
call correlation returns a closed unresolved/integrity result without external entry.

Use the existing `mfm-signing` and EVM nonce-storage primitives. Adding an internal workspace
dependency is permitted only when it replaces the current disconnected implementation; add no
third-party dependency. Production endpoint discovery, credential loading, database provisioning,
key-custody actor construction, process supervision, and concrete JSON-RPC topology remain out of
scope. The Keystore stays `!Send + !Sync`.

### 10. Reduce App to admission orchestration

`Application` owns exactly one mandatory catalog-wide `Runtime`, the cloneable read/config/audit
ports, the two typed resolved configurations/heads, and the secret-free exact planning bindings.
The Store mutation port belongs only to Runtime.

Construction checks same catalog, Store identity/brand, configuration ownership, exact two-entry
closure, and route/registration closure. It is impossible to construct an empty or partially live
App.

Admission dispatch is exhaustive:

- Portfolio: decode selector -> domain plan -> qualify C0/Program -> Runtime admission;
- EVM submission: decode request -> domain plan -> qualify C0/Program -> Runtime admission.

Drive and suspended-owner recovery always use Runtime. Read, replay, audit, and export load the
retained Program callback-free. App does not author a declaration, hash an implementation/target
identity, inspect a domain continuation, interpret evidence, or directly append an admission.

Remove actionable CLI/REST demo paths per Material uncertainty 3 unless separately supplied a real
trusted `Application`. Never replace `Vec::new()` with a fake, null, scripted, or default provider.
Scripted providers exist only in tests.

## Dependency and public-surface changes

- `mfm-program`: owns/re-exports `State`, `FailureValue`, and stable State implementation-ref
  derivation; owns dedicated Program-document identity/ingress.
- `mfm-runtime`: depends on Program's State contracts; owns consuming live implementations,
  catalog-wide assembly, affine sessions, and the sole process registry.
- `mfm-evm`: adds its existing allowed dependency on `mfm-program`; owns EVM State marker types,
  pure transitions, intent preparation/evidence interpretation, failure values, and fragment/entry
  authoring.
- `mfm-portfolio`: depends on `mfm-program`; owns trusted snapshot planning, Portfolio State types,
  EVM-fragment composition, failure mapping, and public consolidation.
- `mfm-evm-live`: binds exact descriptors to providers, nonce authority, and signer interfaces; it
  contains no operation semantics.
- `mfm-app`: consumes the domain planners and one mandatory Runtime; it contains no Program builder
  or live adapter factory.
- Store/Replay remain callback-free and never depend on Runtime or domains.

Do not create another crate to hold operations or composition.

## Required deletion inventory

Delete in the same commit that moves each responsibility:

- App `evm_submission_program`, `portfolio_program`, `PortfolioSourcePlan`,
  `PortfolioCollectionPlan`, `pure_state`, `read_state_to`, `effect_state_to`, address/ref hashing
  helpers, and every ordinal-formatted State implementation identity;
- App-manufactured adapter/physical-target/effect-domain refs;
- App `BTreeMap<EntryPoint, Runtime>`, `Vec<Runtime>` construction, supported-entry compatibility
  loop, direct Store admission/drive fallback, `ApplicationSuspended::Admission`, and
  `document_for_run`/C0 Program reconstruction;
- RuntimeAssembly's Program field, entry-point/program getters, one-registration-per-declaration
  check, and free caller-supplied implementation refs;
- Runtime `Pure`, `Read<C>`, and `Effect<C>` marker structs and borrowed/prebuilt-outcome APIs that
  prevent a consuming handoff;
- `EvmBalanceStage`, `EvmNativeBalanceInput`, `EvmTokenBalanceInput`, source-only Match payloads,
  redundant source/collection cursor and suffix fields, and incomplete generic read strings;
- `ReadWalletNonceStatus`, candidate-before-derivation context construction, incomplete
  `BroadcastIntent`, and unused wallet completion state unless commit 1 gives it an owner;
- EVM/Portfolio failure States that fabricate or claim the success output contract;
- duplicated identity fields in `EvmAdapterBinding`;
- caller-supplied Portfolio collections/routes and dormant Portfolio routing DTOs not used by the
  settled trusted planner;
- `Vec::new()` binary runtime wiring and known-gap text describing structural-only App execution;
- any alternate decoder, alias, feature, fallback, or compatibility re-export discovered by the
  scoped source scan.

Extend `SINGLE_TRUST_BOUNDARY_CUTOVER_MANIFEST.md` only with exact retired identifiers/literals.
Do not add generic forbidden words that would reject legitimate domain nonce reservation or
provider observation terminology.

## Ordered logical commits

Every commit must compile, pass its focused verification, and leave one coherent current design.
If a proposed split would require old and new execution paths to coexist, keep that cutover in one
commit.

### Commit 1: `freeze evm and portfolio public result contracts`

- Resolve all Material uncertainties.
- Check in exact canonical public goldens and the input/evidence/field-ownership table.
- Record the exact submission post-broadcast State suffix.
- Record the standalone binary disposition.
- Update `docs/design.md` only if the intended product contract deliberately differs from the
  normative preservation requirement.
- No execution implementation starts in this commit.

### Commit 2: `persist exact admitted programs`

- Give Program documents a dedicated content identity.
- Include canonical Program bytes in admission object closure and portable export.
- Make Store/Replay/reader paths ingress the retained Program and validate its ref/catalog.
- Make Runtime hot/suspended owners retain the selected Program.
- Delete App Program reconstruction from `C0` and current code.
- Add missing/corrupt/substitution/foreign-catalog/current-code-change recovery tests and update
  Program/Journal/Store/Replay/Runtime documentation.

This remains coherent with the current exact-Program Runtime while permanently fixing cold resume;
it introduces no alternate resume path.

### Commit 3: `execute domain owned evm and portfolio programs`

This is the inseparable vertical cutover:

- move `State`/`FailureValue` to Program and make handoffs consuming;
- make assembly catalog-wide and registrations reusable;
- allow ordinary declared failures to terminate;
- land EVM submission/balance and Portfolio State contracts, transitions, planning, authoring,
  failure mapping, and consolidations;
- fix full-context Match and result metadata;
- register exact EVM read/nonce/broadcast/confirmation adapters and real binding identities;
- make App require one Runtime and typed planning inputs;
- delete all App authoring, optional Runtime, fabricated binding, structural drive, obsolete
  context, incomplete nonce, and fake failure paths;
- remove standalone actionable binary surfaces unless commit 1 selected a real external
  composition;
- update architecture/design, App/domain/live-adapter, transaction/routing/snapshot, and binary
  documentation in the same commit.

Do not split this into “new path” and later “delete old path” commits.

### Commit 4: `record evm portfolio completion evidence`

- Run the final deletion/API/LOC/dependency audit.
- Update capacity fixtures if final Program/C0/Cn sizes changed.
- Append a corrective entry to `IMPL_PLAN_RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.log`; never rewrite
  earlier history.
- Record exact commands/results, public/dependency/LOC deltas, deployment exclusions, and remaining
  limitations.
- Contain no code fix. If evidence finds a defect, fix it in a preceding logical commit and rerun
  evidence.

## Verification plan

### Contract and domain tests

- canonical goldens for frozen EVM submission, EVM collection metadata, Portfolio snapshot/report,
  and typed failures;
- hostile decode tests for every `EvmBalanceWork` variant and every completed/active/remaining
  inconsistency;
- exact native and token stage order;
- Match payload canonical bytes/content identity equal the complete pre-Match successor context;
- non-`Clone` Portfolio continuation moves once through Pure, Access success/failure, Match, EVM
  completion, and Portfolio resume;
- consolidation rejects wrong/common-anchor mismatch, duplicate/missing/foreign/reordered sources,
  wrong binding, decimal overflow, integer overflow, and metadata mismatch;
- Portfolio entry/resume/final consolidation preserves collection order and canonical output;
- failure injection at every EVM stage proves zero preparation/provider activity for all later
  stages, sources, and collections.

Put substantial matrices in named `tests/` targets rather than growing the existing large inline
modules.

### Program, Store, and Runtime tests

- admission contains the exact canonical Program object and matching ref;
- omission, corruption, substitution, wrong schema/ref/entry, and foreign catalog fail cold Store
  selection, replay, export, and Runtime resume;
- changing current planner code/config cannot alter an admitted run's recovered graph;
- one semantic State registration serves repeated unrolled occurrences;
- missing, duplicate, wrong-mode, wrong-contract, wrong-binding, wrong-target, wrong-signer, and
  wrong-effect-domain closure fails before callbacks/provider entry;
- one catalog-wide Runtime executes both entry points and differently shaped Program fixtures;
- Pure/Access input consumption works with a non-`Clone` type;
- terminal typed failure requires no fake failure State and invokes no later work;
- existing affine-owner and `CommittedCall` Trybuild tests remain green.

### Live/App end-to-end matrix

Use the same public composition path as a trusted embedding with bounded scripted provider, nonce,
and signer test doubles:

- both supported entry points reach durable terminal or typed failed outcomes;
- native/token and multiple ordered collection paths record the exact State/provider trace;
- provider/nonce counters are zero before durable preparation and exactly one for the direct-new
  winning call;
- found, stale, acknowledgement-unknown, cold-resume, replay, and history-only branches perform no
  extra provider or State callback;
- nonce acknowledgement-unknown parks without a second reservation;
- every broadcast intent field and signer/target/effect identity is correlation-tested;
- no alternative candidate or transaction is constructed after preparation;
- App construction without the exact Runtime/config/binding closure is impossible;
- a secret canary is absent from Program, C0/Cn, intents, evidence, frames, objects, facts, public
  results, errors, trace, export, and debug output.

Add the missing Keystore `!Send` and `!Sync` compile-fail proofs. Also retain the Send provider-future
proof; do not wrap Keystore in `Arc<Mutex<_>>`.

### Scope-driven commands

During implementation, use the Nix development shell and the narrowest affected commands:

```text
nix develop -c cargo fmt --all -- --check
nix develop -c cargo test -p mfm-program
nix develop -c cargo test -p mfm-store
nix develop -c cargo test -p mfm-runtime
nix develop -c cargo test -p mfm-evm
nix develop -c cargo test -p mfm-portfolio
nix develop -c cargo test -p mfm-evm-live
nix develop -c cargo test -p mfm-app --test evm_portfolio_cutover
nix develop -c cargo clippy -p <affected-package> --all-targets --all-features -- -D warnings
nix develop -c cargo check -p mfm -p mfm-rest-api
nix run .#run -- --task negative-scan
git diff --check
```

Use actual test target names after commit 3 creates them. Do not add a Nix task. If `nixfied.nix`
remains unchanged, do not run `.#model-check`; if it changes, run `nix run .#model-check` early.

On the final coherent code revision run exactly:

```text
nix run .#ci
git diff --check
git status --short
```

Do not immediately precede `.#ci` with `.#check`, `.#test`, `.#test-db`, or individual capacity
leaves; `.#ci` already composes them.

## LOC and complexity budget

The baseline tree contains 35,064 physical lines under `bin/**/src/**/*.rs` and
`crates/**/src/**/*.rs`, and 951 lines matching the repository's public-declaration audit pattern.
Recompute both from the pinned baseline and final tree; do not rely on working-copy-only counts.

Hard constraints:

- no new crate, third-party dependency, Nix task, scheduler, cache, per-input Runtime, generic
  context, planner/factory trait, second registry, per-stage adapter hierarchy, or compatibility
  path;
- the total public-declaration count does not increase, and public type declarations decrease by at
  least two;
- production additions are at most 1,400 lines;
- production deletions are at least 750 lines;
- production net target is `<= +300` lines and the hard ceiling is `+650` lines;
- tests and contract documentation are reported separately and do not justify duplicate product
  abstractions;
- validation, negative tests, and redaction checks may not be deleted merely to meet the budget.

Expected deletion pays for the real semantics: roughly 550 lines of App Program construction plus
the optional Runtime/fallback/reconstruction branches, duplicated adapter identity fields,
redundant context fields/types, fake failure routes, incomplete nonce flow, and non-executable demo
transport code.

After every code commit report product/test/docs `--numstat`, public-declaration delta, and any new
dependency edge. If net product code exceeds `+300`, first remove duplicate stage wrappers,
registration boilerplate, stored derived cursors/suffixes, and assembly products. Crossing `+650`
blocks completion and requires an explicit plan amendment.

## Acceptance criteria

The corrective refactor is complete only when all of the following are true:

- the exact input-planned Program is durably retained at admission and every cold consumer uses it
  without replanning;
- one catalog-wide Runtime and one live registry serve both entry points;
- every EVM/Portfolio State has a domain-owned semantic contract and production implementation;
- repeated source/collection occurrences reuse stable implementations;
- the selected native/token Match arm receives the exact complete cumulative context;
- the Portfolio continuation crosses EVM byte/content-identically and is interpreted only by
  Portfolio;
- EVM balance contexts retain checked chain/anchor/decimal/balance work and final consolidation
  enforces order, binding, common anchor, uniqueness, and checked arithmetic;
- Portfolio planning accepts only selector intent and derives C0/Program from typed trusted config
  and exact binding descriptors;
- all failures stop later normal work and no failure State fabricates a success output;
- nonce reservation is honestly classified as EntryOnce Effect and broadcast intent fixes all
  transaction/signer fields before provider entry;
- any claimed inclusion/disposition is supported by explicit authenticated Read evidence;
- adapter request bytes derive only from committed intent and exact binding identity;
- App cannot be built without the complete live Runtime/config/binding closure and never appends
  directly through a fallback;
- no standalone binary admits actionable work without real trusted composition;
- historical public canonical results match the commit-1 goldens, unless the product contract was
  deliberately changed in the same plan/design revision;
- secret, affine-owner, cold-resume, fail-fast, capacity, deletion-scan, and end-to-end tests pass;
- the final LOC/public/dependency report satisfies the hard budget;
- the implementation log explicitly supersedes the earlier EVM/Portfolio completion claim and
  records the final `.#ci` evidence.
