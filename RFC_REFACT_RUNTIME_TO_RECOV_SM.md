# RFC: replace the graph Runtime with a recoverable linear state machine

Status: proposed. This RFC records the replacement design; it does not change the current
implementation contract in [docs/design.md](docs/design.md) until the implementation cutover.

## Decision

Replace the current forward-only State/Match execution model with one immutable, ordered sequence
of typed States and a Runtime-owned recovery state machine.

Configuration determines which States exist before admission. When that information requires IO,
a separate enrichment run produces a checked immutable value that Application uses to plan the
dependent run. Once admitted, a Program never grows or changes.

Normal success advances to the next State. A typed deterministic failure policy requests retry of
the failed State, restart from an explicit checkpoint, or terminal failure. Runtime validates the
request, applies budgets and Effect barriers, and persists the outcome and recovery decision
atomically. Recovery does not require authors to duplicate State sequences or construct branches.

This is a complete replacement, not another execution mode. Remove Match, arbitrary success and
failure edges, graph failure handlers, and the old graph compiler/fold. Preserve the useful
boundaries: typed deterministic States, explicit IO capabilities, immutable content-addressed
Programs and values, one Runtime fold, exact append-only Journal history, and mechanical Store IO.

## Motivation and evidence

The current Program is a forward-only State/Match graph. It executes one selected State at a time,
but cannot revisit a completed declaration. Consequently, authored recovery can move forward into
a handler but cannot directly express retrying a State or restarting a completed region.

The graph makes static branch validation and once-per-declaration frame bounds convenient. Those
benefits do not justify making ordinary recovery an expansion of alternative business paths.

The existing balance example does not require runtime branching. Portfolio planning already has
each source's optional token address, but passes only its source count into `CollectEvmBalances`.
Expansion then emits `SelectBalanceAsset` and both native/token branches. See the
[Portfolio planner](crates/domains/portfolio/src/lib.rs) and
[EVM Operation](crates/domains/evm/src/lib.rs).

Instead, expansion receives the checked ordered source plan and emits only the applicable States:

```text
native: check chain -> read anchor -> read native balance -> confirm anchor
token:  check chain -> read anchor -> read decimals -> read token balance -> confirm anchor
```

A mixed portfolio concatenates the selected sequences in source order. Repeated expansion of the
same exact configuration and implementation produces the same Program bytes and reference.

## Goals and explicit exclusions

The design must make the normal execution path readable as a list, make recovery reusable rather
than authored as topology, and retain deterministic hot/cold interpretation of every committed
transition. It must support bounded retries and restart of a Read region without erasing history
or accidentally repeating external actions.

The initial replacement deliberately excludes:

- Runtime business branching, arbitrary jumps, and dynamic Program expansion.
- Background retry workers, durable timers, and a second workflow scheduler.
- Implicit compensation, repetition of settled Effects, and abandonment of unresolved Effects.
- Mutation or truncation of acknowledged history, legacy decoders, and mixed-runtime execution.
- Treating every implementation or infrastructure error as a recoverable domain failure.

A later business action whose shape depends on an execution result requires a separately planned
Program. This is an intentional expressiveness boundary, not an invitation to hide a scheduler or
multiple IO actions inside a State.

## Ownership

| Owner | Target responsibility |
| --- | --- |
| Values / IDs | Checked value schemas, context reconstruction, content refs, run and Effect identities. |
| Program | Immutable ordered State declarations, exact contracts, checkpoint declarations, recovery policy identities and bounded parameters; deterministic linear authoring. |
| Domain | Pure State semantics, typed failure classification, meaningful checkpoint selection, resolved planning values and enrichment validity rules. |
| Runtime | Exact assembly association; sole semantic fold; execution position, visits, checkpoints, budgets, Effect barriers, and authorized recovery transitions. |
| Capabilities / adapters | Typed Read observations and Effect settlement; explicit async IO and duplicate-safe recovery of existing Effects. |
| Journal | Exact canonical frames, object closure, append-only wire qualification, and Effect prepare/conclusion structure. |
| Store | Complete-prefix load and atomic exact-head append only. No recovery, checkpoint, or domain semantics. |
| Application | Validate and bind configuration; coordinate explicitly selected enrichment and dependent admission; expose typed run use cases. |
| CLI / REST | Parse, call Application, and render its redacted contract. |

An Operation remains authoring-only. Runtime never receives an Operation instance, expansion
callback, provider discovery callback, or mutable configuration service.

## Configuration, enrichment, and admission

### Static configuration

The Operation receives enough checked immutable configuration to choose the complete State list.
This includes ordered native/token source shapes for balance collection. Planning produces the
Program and its exact initial context together and checks their agreement. Runtime and State
preparation retain local validation so a mismatched supplied context cannot enter a provider.

Public metadata that affects topology is part of the checked planning input. Credentials, signer
handles, provider clients, and filesystem/database locators remain process-local capabilities.

### Enrichment requiring IO

```text
user configuration
    -> enrichment Program admission and execution
    -> completed typed discovery output
    -> Application validation and binding
    -> deterministic dependent Operation expansion
    -> dependent Program admission and execution
```

Enrichment is ordinary execution with journaled Reads and the same recovery rules. Its output is
a typed value, not an ambient mutable map. Application checks schema, bounds, provenance, binding,
and any domain-required freshness before using it. Token decimals remain anchored execution
Reads unless an explicit domain contract permits treating them as frozen planning metadata.

Each admitted stage has its own explicit RunId and immutable Program. The dependent initial
context retains the exact resolved planning value and, when discovery is used, a bounded reference
to the successful enrichment run/head/output. Application verifies that linkage before admission.
That linkage is provenance, not automatic authentication of arbitrary imported provider claims.

Persisting the resolved value as a named configuration revision is optional. Admitted runs retain
everything needed for cold recovery and do not require the original configuration revision to
remain in the repository.

The initial API exposes the stage boundary explicitly. Callers can resume enrichment, read its
terminal output, and request dependent admission with its own stable RunId. An ambiguous admission
is resolved with that same identity and exact input. No hidden automatic cross-run orchestration
or crash-recoverable parent workflow is introduced by this RFC.

The asset list means "collect these resolved assets." It does not automatically promise discovery
of every asset held at a later snapshot. Stronger discovery/snapshot consistency belongs to an
explicit domain contract and its evidence tests.

## Linear Program and authoring

A Program contains an ordered, bounded list of typed Pure, Read, and Effect declarations. Each
declaration names its exact implementation/input/output/failure ABI and relevant capability binding.
Its normal success continuation is the next list element; the last success produces the declared
root output. An empty Program is permitted only for the checked identity-success contract.

Construction checks adjacent output/input equality, root contracts, complete capability association
requirements, policy contracts, checkpoint references, and numeric bounds. There are no arbitrary
successor indices, selector declarations, branch payload projections, or public graph draft.

Operations compose child Operations by deterministic concatenation through `OperationExpansion`.
Capability injection may still contribute an ordered before/designated/after sequence. Its hooks
remain pure authoring and the designated occurrence is inserted exactly once. Original failures
do not execute an after hook as a finally action. Recovery declarations are validated after the
complete injected sequence is known, including every injected Effect barrier.

Checkpoint handles are scoped authoring references that lower to checked State positions. Child
composition cannot capture arbitrary parent positions. A parent may establish a recovery region
around a child; validation checks the complete expanded region and compatible checkpoint input.
There is one linear authoring implementation, not parallel graph and sequence APIs.

## Recovery policy

The names below describe contracts, not a frozen Rust API:

```text
on typed domain failure:
    RetryState
    Restart(checkpoint)
    Fail(root_failure)
```

Each recoverable step associates one deterministic typed policy with a versioned implementation
identity and immutable bounded parameters in the Program. Runtime assembly associates its exact
ABI, just as it associates State implementations. Policies consume the typed failed outcome and
the explicitly admitted context/counter information they require; they perform no ambient IO,
read no clock, and receive no Store or adapter authority.

Policy composition must map each State's failure into the Operation's one root failure contract.
The same contract provides a deterministic root failure when recovery is exhausted or disallowed
by an Effect barrier. Its exact ABI includes the original typed failure and the bounded reason
recovery was denied or exhausted. There is no erased error bag or generic catch-all handler Operation.

Runtime owns authorization: a policy cannot bypass mode restrictions, restore arbitrary values,
reset counters, or authorize another Effect. Structurally invalid policy results are internal
implementation errors. Expected exhaustion or an unavailable checkpoint produces the declared
terminal failure through the typed policy contract.

Defaults are terminal domain failure and zero recovery allowance. Recovery is explicit. Classifier
and failure-mapping code is associated implementation code, never serialized executable code.

### Domain failures versus execution errors

- A typed domain failure may be classified and durably retried, restarted, or terminated.
- Preparation, interpretation, policy callback, local binding, and invariant failures return a
  redaction-safe Runtime error without manufacturing a domain conclusion.
- Adapter unavailability, Store errors, and cancellation do not enter domain classification.
- Ambiguous append acknowledgement requires reload; it is never evidence that nothing committed.

For an internal Pure/Read callback error, the current durable head remains unchanged. For an Effect
callback error after preparation, its exact acknowledged prepare remains pending. The next caller
can resume that retained position. Cold fold never reruns a completed classifier or interpreter.

## Checkpoints and restart semantics

A checkpoint names the input boundary before a State. Its active snapshot is the exact immutable
input at the most recent entry to that checkpoint on the current execution path. Genesis provides
the initial boundary. A checkpoint references retained qualified values; it is not a mutable Store
snapshot or a second persistence service.

An activation is identified by the genesis or committed transition head that established it,
checkpoint position, destination visit, and exact input reference. Merely invoking `resume` cannot
create another activation. A restart decision names the prior eligible activation and input; the
committed restart deterministically establishes its replacement without an extra checkpoint frame.

Restart restores that exact input and resumes at the checkpoint's State. Later derived values and
checkpoint activations cease to be active, but all their historical frames remain unchanged.
Entering a checkpoint again establishes its current activation. Recovery counters are run-scoped
and never roll back with the context, so nested restarts cannot reset their allowances.

Domain authors choose recovery regions. Pure/Read/Effect modes constrain safety but cannot infer
the correct region. For example, an anchor mismatch must restart from the Read that established
the anchor, not repeatedly confirm a snapshot already known to be inconsistent.

```text
checkpoint collection
read anchor -> read balance -> confirm anchor
      ^                              |
      +------ Restart(collection) ---+
```

This return is a Runtime recovery transition. It is not a business edge in the Program.

### Mode rules

| Mode | Retry and restart contract |
| --- | --- |
| Pure | Repeating a typed deterministic failure with the same input cannot repair it. Reject `RetryState`; allow restart only through an eligible earlier Read region or terminate. Interrupted pure work may be recomputed. |
| Read | `RetryState` starts a new visit with the same exact input and may obtain new evidence. Restart can re-observe an eligible earlier region and recompute its suffix. |
| Effect before acknowledged prepare | No external authority has been entered. Preparation errors retain the current position; no fabricated settlement or domain recovery occurs. |
| Effect with acknowledged prepare | Hard recovery barrier. Resume only its retained command and EffectId until settlement. No checkpoint jump may abandon it. |
| Settled Effect | Its external outcome remains real. Success advances; typed domain failure terminates under this initial contract. Never implicitly execute it again. |

After successful Effect settlement, recovery may use checkpoints in the subsequent Pure/Read
suffix. It cannot restore a checkpoint before that Effect. The post-Effect input boundary may be
explicitly declared as a checkpoint. Reusing earlier completed Effect facts means retaining them
in that input, not executing the Effect again.

The same rule applies to injected reservation and preparation Effects. They are Effects even when
they do not broadcast a transaction. A later Pure projection of settled failure cannot restart
across them. Compensation or a genuinely new transaction requires a separately authored action
and fresh admission; it is outside generic rollback.

## Runtime progression, budgets, and identity

Runtime has one fold over retained history. Its active state distinguishes a runnable State visit,
an exact pending Effect, and terminal success/failure. It also derives active checkpoint values and
monotonic recovery counters. Avoid correlated status flags and independent mutable cursors.

A State position identifies a definition within a Program. A visit identifies one execution of
that position. A committed retry or restart selects a new visit; it does not overwrite a prior
outcome. Visit numbering is deterministically derived from qualified transitions and checked for
overflow. An interrupted invocation without a committed transition does not invent another visit.

EffectId derivation binds RunId, Program reference, State position, visit identity, and exact
command reference under the new canonical domain. Resuming a pending Effect always reuses the
same visit, command, and EffectId. Invocation count is not Effect identity. This provides durable
authorization and reconciliation, not a generic exactly-once guarantee about external systems.

Each admitted policy has finite retry/restart allowances and the Program has a finite total
recovery-decision bound. Runtime enforces the global bound as well as local allowances. Exhaustion
produces the typed terminal failure atomically with the failed attempt that exhausted recovery.

A committed retry or restart yields a Runnable view. A later `resume` executes the selected visit.
Successful steps may continue in the current invocation; a pending Effect also yields. This keeps
progress caller-driven without tight retry loops, clocks, sleeps, or background tasks in Runtime.
Clients may control pacing outside State semantics.

Infrastructure failures and repeated pending observations that append no frame do not consume a
durable domain-recovery budget. Therefore the bound is on committed recovery decisions, not wall
time or total provider invocations. Runtime makes no eventual-settlement guarantee.
Exhausted recovery allowances cannot revoke an acknowledged pending command: its retained visit
must remain eligible for settlement and its reserved conclusion capacity remains available.

## Journal, atomicity, and replay

Introduce one current linear-recovery Program and Journal wire contract with new identities. Old
graph bytes are rejected. Do not reinterpret them as sequences or add compatibility readers.

Pure/Read conclusions retain the typed outcome; Read conclusions also retain exact intent and
accepted evidence. A failed conclusion additionally carries the authorized recovery disposition,
target checkpoint activation where applicable, and sufficient exact references to qualify it.
Effect prepare retains the command, visit, and EffectId; its adjacent conclusion retains bound
settlement and the terminal-or-advance disposition.

Failure and recovery decision are one atomic conclusion. There is no durable intermediate failed
State waiting for an unrecorded classifier choice. If classification or validation fails before
append, no conclusion is committed. An Effect's prepare remains pending in that case.

Journal owns structural wire qualification and complete frame-local object closure. Runtime owns
semantic validation against the associated Program: expected visit and position, allowed mode,
checkpoint eligibility and input, counters, and Effect barriers. Cold fold validates retained
decisions without re-executing policy callbacks, State interpretations, or providers. A retained
pending Effect is re-prepared only to validate its exact command and identity, as today.

Store remains mechanical. Every append is exact-head and all-or-nothing. A losing concurrent
append reloads the winning complete history. Concurrent recoveries cannot each spend the same
budget or establish competing active snapshots. Concurrent Effect invocations must remain safe
through the adapter's same-identity protocol; Store locking alone does not prevent duplicate IO.

### Capacity

Keep explicit finite canonical-object, frame, frame-count, and total-history byte limits. The
current once-per-declaration formula is removed because declarations may be revisited. Replace it
with checked admission arithmetic proving a conservative maximum for the complete sequence and
all executions permitted by its recovery bounds, in both frames and bytes. Include genesis,
successful suffixes, failed attempts, checkpoint value closure, and terminal dispositions. Reject
admission before genesis if the full bound exceeds the format limits. There is no extra durable
capacity-stop state and no requirement that a `Never` State manufacture a domain failure.

Each step's admitted size bounds must cover its complete frame-local closure, including evidence,
context, and policy output; Runtime enforces those bounds on returned values. Planning may use
tighter checked bounds than the global maximum object size, but cannot omit a possible valid
execution within the admitted contract. Exact runtime accounting before every append checks the
admission invariant. Never let a retry reset history accounting.

Before preparing an Effect, Runtime must reserve enough remaining frame/count/byte capacity for
both its prepare and its maximum admitted conclusion, including terminal failure disposition.
The pending prepare must retain the bound needed to validate that reservation on cold load.
Evidence larger than the capability's admitted bound is rejected; it cannot consume the reserve.
No adapter is entered for an Effect whose complete bounded durable lifecycle cannot fit.

For Pure/Read execution, check available conclusion capacity before IO where IO applies. A retry
or restart must not authorize a visit that has no capacity for its required bounded conclusion.
Capacity admission and error behavior must be covered at the limit, including a pending Effect
near the history bound. Do not claim that this storage guarantee ensures external settlement.

## Public behavior

Keep one typed start/resume/read surface. `read` folds without execution. `resume` progresses a
runnable visit or reconciles the exact pending Effect; terminal resume is unchanged and performs
no execution. Runnable recovery exposes a bounded reason and target State position so callers
can distinguish retry/restart from pending settlement without inspecting frames.

Expose terminal domain failure separately from redacted Runtime errors. Preserve stable recovery
identities on ambiguous admission/progress acknowledgements. No secrets, raw provider errors,
configuration locators, or arbitrary classifier diagnostics enter public output.

Application owns mapping into shared client models; CLI and REST update together with their
documented transport asymmetries. They do not implement retry policy or derive run status.

## Complete replacement and deletion scope

The implementation must replace all current producers and consumers together:

1. Program wire, checked construction, and authoring: remove `Declaration::Match`,
   `MatchDeclaration`, `MatchVariant`, `MatchJoin`, `match_join`, `with_failure_handler`, arbitrary
   successor edges, graph joins/frontiers, selector projections, and obsolete graph-only helpers.
   Replace them with sequence construction, scoped checkpoints, and exact typed recovery policy.
2. Runtime assembly and engine: delete graph association, Match selection, edge advancement, and
   once-per-declaration assumptions. Implement the sole linear recovery fold and associated policy
   callbacks. Keep one public Runtime API and no old engine behind flags.
3. Journal and IDs: replace the wire/qualification fixtures and EffectId derivation; update all
   complete-history, hostile-input, capacity, and exact-byte tests. Store gets no semantic API.
4. Domains and live composition: specialize balance expansion from ordered sources; remove
   `SelectBalanceAsset` and its selector-only sum/metadata/registrations where obsolete. Replace
   Portfolio failure-handler topology with typed root-failure mapping and declared recovery policy.
   Preserve chain/route/input validation and anchored evidence contracts.
5. Effect injection and custody consumers: update the four-State transaction sequence, identity
   users, fixtures, and pending recovery tests. Retain exact prepared-wire reuse and signer-free
   recovery guarantees. Do not promote the development finality contract into production.
6. Application, CLI, REST, discovery inventories, examples, and contract snapshots: remove graph
   assumptions and expose the one current run model. Add explicit enrichment-to-admission use
   cases without adding a hidden scheduler.
7. Documentation: replace the current contract in `docs/design.md` and taxonomy in
   `docs/architecture.md`, update affected binary READMEs and rustdoc, and align verification docs
   and executable task descriptions with the new contracts.

Follow the repository clean-slate policy: reset affected development baselines and reject old
persisted contracts. Never truncate or rewrite acknowledged histories in place. No legacy schema
reader, data migration, shadow execution, dual API, or deprecated graph implementation remains.
Unrelated config/Store formats need not change solely because Runtime changes.

Review transaction reservation and prepared-wire custody keys together with the EffectId change.
For the breaking development cutover, provision a fresh transaction-authority epoch and fresh
RunIds; retain or retire prior authority under its existing contract. Never automatically resubmit
an old action under a new identity, delete custody to force a retry, or assume an old unresolved
Effect did not execute. Retire existing unresolved runs operationally before switching their
deployment to a Runtime that rejects their format. This RFC specifies no live-data migration.

## Acceptance tests

Tests must use public boundaries and independently specify observable behavior:

- Native-only, token-only, and mixed configuration produce the selected linear sequences and
  correct balances; mismatched Program/input fails before provider entry.
- Expansion is deterministic; incomplete adjacent types, invalid checkpoint scopes, incompatible
  failure policies, and unbounded policies fail construction or association.
- A Read domain failure retries with the same input and new evidence; a Pure deterministic failure
  cannot select same-input retry.
- Anchor mismatch restarts the declared collection region, discards active suffix values, retains
  prior frames, and yields the same result and head interpretation after cold reload.
- Repeated/nested restart cannot reset local/global budgets; exhaustion produces terminal failure.
- Cancellation and ambiguous append at every recovery boundary converge on one qualified history;
  concurrent callers cannot spend one recovery allowance twice.
- Classifier/internal errors append no conclusion; cold fold never calls completed interpreters,
  classifiers, providers, or signers.
- Pending Effect recovery keeps the exact command/EffectId, cannot jump to a checkpoint, and does
  not rebroadcast a new action after settled success or failure.
- A checkpoint after an Effect can restart its Read suffix; a checkpoint before it is ineligible,
  including when the Effect was introduced by injection.
- Exact wire/hash vectors, malformed decisions, forged checkpoint refs, stale visits, budget
  overflow, invalid Effect adjacency, and old graph wire are rejected by the correct owner.
- Capacity is checked before Effect authority, with enough reserved space for a maximum admitted
  settlement at the bound; all histories remain within fixed limits.
- Enrichment output linkage is exact; a resumed dependent run does not rediscover assets or depend
  on a deleted config; invalid/stale discovery fails admission under its declared domain policy.
- CLI/REST agree on recovery state and identities while preserving documented transport behavior.

## Ordered implementation commits and verification

1. **Record the proposal.** This RFC only; review assumptions without changing current contracts.
2. **Replace the execution contract completely.** Keep the inseparable Program/Runtime/Journal/
   Effect-identity cutover, all dependent domain/adapter/application/binary changes, deletions,
   fixtures, tests, and current design documentation in one coherent logical commit. Do not split
   it by crate if doing so requires an intermediate dual design or broken consumer.
3. **Add live enrichment workflows.** Once the linear contract exists, add concrete discovery
   Operations and explicit dependent-admission use cases with provenance/validity tests and docs.
   Static configured portfolios already work after the replacement commit; enrichment is not a
   fallback to the old Runtime.

Use lower-case commit subjects. Follow [code quality](docs/code-quality.md) and the scope-driven
[verification guide](docs/build-and-verification.md). The RFC-only commit needs relative-link
review and `git diff --check`; it does not select Rust gates.

For the implementation cutover, run focused Program/Journal/Runtime tests first, then affected
domain, live adapter, application, and transport tests inside the default Nix shell. Exercise the
managed PostgreSQL, client recovery, Effect recovery, and capacity boundaries through their
documented tasks as selected by scope. Run one final `nix run .#ci` on the exact implementation
candidate; do not redundantly run its broad component gates immediately beforehand.

## Alternatives considered

- **Retain Match and add recovery transitions:** supports runtime business branching but keeps
  branch authoring, graph validation, and two control concepts without a demonstrated requirement.
- **Unroll bounded retries into a forward graph:** duplicates business work and makes recovery
  limits change Program topology. Rejected for maintainability and authoring complexity.
- **Let States return arbitrary next-State IDs:** moves scheduling into domain implementations,
  weakens typed composition, and recreates an implicit graph. Rejected.
- **Perform live discovery inside expansion:** makes Program construction depend on ambient IO
  and provides no ordinary durable recovery for that IO. Rejected.
- **Append States to an admitted Program:** changes immutable Program identity and introduces a
  second admission protocol within a run. Separate enrichment/admission is the selected boundary.
- **Implement rollback by truncating history:** loses acknowledged evidence and cannot undo
  external actions. Rejected.

## Material uncertainties

These are explicit proposed assumptions, not unspecified implementation choices. Review them
before implementing the replacement.

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Enrichment and dependent execution have separate RunIds and an explicit API boundary. | The discussion approved enrichment before expansion but did not settle lifecycle identity. | A single resumable parent request would require a separately designed durable coordinator; it must not be smuggled into Runtime. | Review discovery completion, crash before dependent admission, and ambiguous admission with the intended client flow. |
| No required product needs runtime business branching within one Program. | Existing native/token selection is static, but future products have not been enumerated. | Some workflows must split at an execution result; unacceptable splits would require revisiting this expressiveness boundary. | Model representative portfolio and transaction workflows using resolved linear sequences. |
| Effects are hard barriers and settled Effect failure terminates. | The requested rollback behavior did not specify compensation or crossing completed Effects. | Recovery involving undo/replacement requires a new explicit domain contract and cannot use generic checkpoint restart. | Walk through failure after reservation, signing, broadcast, and settlement. |
| Retry/restart yields for caller-driven progression, without durable timers. | Automatic pacing/backoff was not specified. | Unattended timed recovery would need an external driver contract. | Validate intended CLI/REST usage and service operation. |
| Discovery freezes a bounded asset list with domain-defined validity. | Exhaustiveness and snapshot freshness requirements are not yet specified. | Stronger portfolio claims require anchored discovery and consistency evidence. | Specify expected results when assets or metadata change between discovery and collection. |
| Explicit bounded policy inputs and typed failure mappings suffice without recovery Operations. | Exact generic Rust authoring signatures have not been prototyped. | Excessive type machinery could undermine the simplification. | Implement a minimal consuming-crate prototype for nested Portfolio/EVM failure mapping and one checkpoint; accept only one compact final API. |
| Conservative full-history admission bounds can fit useful portfolios and recovery allowances. | Per-step complete-closure bounds have not yet been measured against the existing capacity fixtures. | Overly broad bounds could reject useful runs; tighter checked contracts or an explicit format-capacity decision would be required. | Prove bounds for the existing 64-source and transaction fixtures with representative retry budgets before freezing the wire. |
