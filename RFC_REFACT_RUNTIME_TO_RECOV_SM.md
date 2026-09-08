# RFC: replace the graph Runtime with a recoverable linear state machine

Status: proposed. This RFC records the replacement design; it does not change the current
implementation contract in [docs/design.md](docs/design.md) until the implementation cutover.

## Decision

Replace the current forward-only State/Match execution model with one immutable, ordered sequence
of typed States and a Runtime-owned recovery state machine.

Configuration determines which States exist before admission. When that information requires IO,
a separate enrichment run produces a checked immutable value that Application uses to plan the
dependent run. Once admitted, a Program never grows or changes.

Normal success advances to the next State. Two independently configurable implementations classify
typed domain failures or structured execution errors and choose recovery. Operations supply
defaults; individual State occurrences may override them. Expansion resolves one effective binding
per declaration. Runtime assesses incidents through those bindings, validates requested recovery,
and applies budgets and Effect barriers. Eligible incidents and their recovery decisions are
persisted atomically. Errors whose durable outcome is unknown or cannot be recorded stop the
invocation without inventing a terminal run. Recovery does not require authors to duplicate State
sequences or construct branches.

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

The simplification is smaller business authoring and one reusable recovery mechanism. Visits,
checkpoint activation, budgets, and recovery qualification add Runtime responsibilities compared
with today's less capable forward-only engine. Configurable classifiers and handlers remove
hard-coded policy choices, not the Runtime's execution-safety checks. The cutover must demonstrate
a compact consuming API; fewer Runtime lines are not an assumed acceptance result.

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
| Program | Immutable ordered State declarations; public classifier/handler authoring contracts; resolved exact bindings, checkpoints, bounded parameters, and typed failure mapping. |
| Domain | Pure State semantics, public typed failures and reusable classification/handling implementations, meaningful checkpoint selection, resolved planning values and enrichment validity rules. |
| Runtime | Public structured execution faults and run reports; exact assembly association; incident assessment; sole semantic fold; visits, checkpoints, budgets, Effect barriers, and authorized recovery transitions. |
| Capabilities / adapters | Public typed Read observations, Effect settlement, and redaction-safe adapter errors; explicit async IO and duplicate-safe recovery of existing Effects. |
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

## Classification and recovery handling

### Public reusable contracts

Domain failures, framework/runtime/adapter errors, classification contracts, recovery-handler
contracts, and default report types are public documented library APIs. Downstream crates can
inspect and reuse typed causes, implement classifiers and handlers, and compose them without the
CLI, private Runtime types, or changes to framework source. Public extension does not grant Store,
adapter, or scheduling authority, and does not permit unchecked construction of qualified values.

Domains own their typed failures. Each framework boundary owns its reviewed error vocabulary;
Runtime preserves origin and execution phase when forming a structured execution fault. Classifiers
must be able to distinguish Read unavailability, pending Effect unavailability, and Store append
uncertainty before transport rendering collapses detail. Raw provider errors, arbitrary diagnostic
strings, and secrets are not classifier inputs or public error payloads.

Program owns the generic authoring/association contracts; Runtime supplies its concrete execution
fault contract at association. Domain implementations need not depend on Runtime to classify their
own failures. These are typed extension points, not a serialized executable or erased error bag.

### Two configurable implementations

The names below describe contracts, not a frozen Rust API:

```text
typed domain failure or structured execution fault
    -> classifier: typed assessment of cause and recoverability
    -> recovery handler: requested action using assessment and admitted recovery context
    -> Runtime: validate, apply budgets/barriers, and commit or stop the invocation

requested actions:
    RetryState
    Restart(checkpoint)
    Stop
```

The classifier assesses the incident; the handler selects recovery. A nonrecoverable assessment
permits only stopping; a recoverable assessment does not guarantee that Runtime can authorize the
requested action. Both components are independently replaceable deterministic implementations with
exact versioned identities and compatible typed ABIs. They consume only the typed
incident/assessment and explicitly admitted bounded parameters,
context, and counter information. They perform no ambient IO, read no clock, and receive no Store
or adapter authority. A pending Effect permits only reconciliation of its existing authority;
`RetryState` cannot create a new visit or command in that phase.

Typed domain failures map into the Operation's one root domain-failure contract. Execution faults
retain their framework identity in the default report and do not require every domain to invent
business failures for infrastructure incidents. Exhaustion and denial use a bounded stop reason
alongside the original cause. There is no catch-all recovery Operation or middleware chain.

### Operation defaults and State-occurrence overrides

Operation expansion may supply defaults for classification, recovery handling, and bounded
allowances. The nearest explicitly supplied Operation default wins for each setting; an explicit
State-occurrence setting overrides that default. A parent default fills unspecified child settings
but does not silently replace an explicit child choice. Configuration belongs to the occurrence,
so one reusable State implementation may have different policies in different Operations.

Expansion resolves inheritance, checks the final classifier/handler/failure-mapping compatibility,
and lowers one complete effective binding per declaration into Program. Injected occurrences use
the same authoring rules. Runtime associates those exact bindings once; it never receives Operation
scopes or resolves dynamic inheritance. Configuration and allowances cannot change after admission.
Framework defaults request no recovery and provide the standard stop report; allowances are zero
unless explicitly configured. A complete binding must cover both the State's domain-failure
contract and applicable framework faults, with reusable framework defaults for the latter.

### Runtime authorization and error boundaries

Runtime uses the configured classifier and handler wherever a trustworthy associated execution
position exists, but retains sole authority over legal transitions. A policy cannot bypass mode
restrictions, restore arbitrary values, reset counters, or authorize another Effect. Structurally
invalid policy results and classifier/handler failures return an internal error without recursively
entering the same recovery pipeline. Expected exhaustion or an ineligible checkpoint produces the
standard stop report; it becomes terminal only where a valid durable conclusion is possible.

| Incident | Authorized behavior |
| --- | --- |
| Typed domain failure | Classify and atomically conclude with mode-eligible retry/restart or terminal domain failure. |
| Read adapter unavailability without accepted evidence | Classify a structured execution fault; atomically record that fault and retry, eligible restart, or terminal execution failure. Never fabricate evidence or a domain outcome. |
| Pending Effect adapter error | Assess the safe fault, but retain the exact prepare, visit, command, and EffectId. Yield for same-authority reconciliation or stop the invocation; never abandon or terminally settle the Effect from an error. |
| Store unavailability | Stop with an operational report if progress cannot be recorded. No durable recovery allowance is consumed without a committed decision. |
| Ambiguous append acknowledgement or a lost exact-head race | Reload the winning complete history before selecting another transition. Uncertainty is never evidence that nothing committed. |
| Preparation, interpretation, local binding, or invariant error | Stop with a safe execution error; no fabricated domain conclusion, new provider call, or recovery append. Policy cannot authorize bypassing the failed invariant. |
| Invalid history, incompatible association, or error before a trustworthy execution position exists | Stop through the public framework report; do not invoke an unassociated policy or execute against invalid history. |
| Cancellation | Preserve committed authority; no guaranteed callback or append during cancellation. A later caller reloads and resumes. |

For an internal Pure/Read callback error, the current durable head remains unchanged. For an Effect
callback error after preparation, its exact acknowledged prepare remains pending. Cold fold never
reruns completed classifiers, handlers, or interpreters. Infrastructure errors that cannot be
durably recorded remain invocation errors, even when a configured handler requests stopping.

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
| Read | A domain failure or eligible adapter execution fault may select `RetryState`, starting a new visit with the same exact input and potentially new evidence. Restart can re-observe an eligible earlier region and recompute its suffix. |
| Effect before acknowledged prepare | No external authority has been entered. Preparation errors retain the current position; no fabricated settlement or domain recovery occurs. |
| Effect with acknowledged prepare | Hard recovery barrier. Resume only its retained command and EffectId until settlement. No checkpoint jump or exhausted invocation recovery may abandon it or manufacture terminal settlement. |
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

A State position identifies a definition within a Program. The tuple
`(RunId, ProgramRef, StatePosition, VisitId)` identifies an execution occurrence. VisitId is one
run-wide monotonic counter, deterministically advanced when a committed transition selects the next
State execution, including retry or restart, and checked for overflow. Retry counters govern
allowances rather than adding another identity dimension. An interrupted invocation without a
committed transition does not invent another visit.

Restart selects a retained checkpoint input and appends to the current Journal head. Execution may
return to an earlier State position, but Journal history remains one linear hash chain; it never
forks from a prior head. Old occurrences and their evidence remain immutable. Visit identity
distinguishes repeated executions; exact-head atomic append decides which competing transition
becomes history. Neither mechanism alone prevents duplicate external IO.

EffectId derivation binds RunId, Program reference, State position, visit identity, and exact
command reference under the new canonical domain. Resuming a pending Effect always reuses the
same visit, command, and EffectId. Invocation count is not Effect identity. This provides durable
authorization and reconciliation, not a generic exactly-once guarantee about external systems.

Each effective recovery binding has finite retry/restart allowances and the Program has a finite
total recovery-decision bound. Runtime enforces the global bound as well as local allowances.
For a domain failure or eligible Read execution fault, exhaustion produces the terminal report
atomically with the incident that exhausted recovery. Local counters belong to declaration
occurrences, remain run-scoped, and never reset when a checkpoint restores input.

A committed retry or restart yields a Runnable view. A later `resume` executes the selected visit.
Successful steps may continue in the current invocation; a pending Effect also yields. This keeps
progress caller-driven without tight retry loops, clocks, sleeps, or background tasks in Runtime.
Clients may control pacing outside State semantics.

Committed recovery for eligible Read execution faults consumes the same admitted allowances as
domain-failure recovery. Store errors, pending Effect observations, and other incidents that append
no frame do not consume a durable recovery budget. Therefore maximum recovery tries means
committed recovery decisions, not wall time or total provider invocations. A Read timeout followed
by an unsuccessful append does not durably spend an allowance; the next caller reloads first.
Each pending Effect reconciliation yields rather than retrying internally. Caller-side limits on
unrecorded invocation attempts are operational limits, not durable counters or terminal run facts.
Runtime makes no eventual-settlement guarantee.
Exhausted recovery allowances cannot revoke an acknowledged pending command: its retained visit
must remain eligible for settlement and its reserved conclusion capacity remains available.

## Journal, atomicity, and replay

Introduce one current linear-recovery Program and Journal wire contract with new identities. Old
graph bytes are rejected. Do not reinterpret them as sequences or add compatibility readers.

Pure/Read domain conclusions retain the typed outcome; Read domain conclusions also retain exact
intent and accepted evidence. A distinct Read execution-fault conclusion retains the exact intent,
reviewed fault origin/phase/code, and recovery disposition, with no accepted evidence or domain
outcome. These alternatives must be represented explicitly, not through a fabricated evidence
value or unrelated optional fields. An incident conclusion additionally carries the authorized
recovery disposition, target checkpoint activation where applicable, and sufficient exact
references to qualify it, including the resulting terminal report where applicable.
Effect prepare retains the command, visit, and EffectId; its adjacent conclusion retains bound
settlement and the terminal-or-advance disposition.

An eligible incident and recovery decision are one atomic conclusion. There is no durable
intermediate failed State waiting for an unrecorded classifier or handler choice. If classification,
handling, mapping, or validation fails before append, no conclusion is committed. An Effect's
prepare remains pending in that case. Pending Effect errors add no intermediate frames between
prepare and settlement conclusion; stop reports for those invocations are not settlement records.

Journal owns structural wire qualification and complete frame-local object closure. Runtime owns
semantic validation against the associated Program: expected visit and position, allowed mode,
checkpoint eligibility and input, counters, fault eligibility, report contracts, and Effect
barriers. Cold fold validates retained decisions without re-executing classifiers, handlers,
failure mappings, State interpretations, or providers. A retained pending Effect is re-prepared
only to validate its exact command and identity, as today.

Store remains mechanical. Every append is exact-head and all-or-nothing. A losing concurrent
append reloads the winning complete history. Concurrent recoveries cannot each spend the same
budget or establish competing active snapshots. Concurrent Effect invocations must remain safe
through the adapter's same-identity protocol; Store locking alone does not prevent duplicate IO.

### Capacity

Keep an independent finite Program declaration/size bound and explicit finite canonical-object,
frame, frame-count, and total-history byte limits. Declaration count bounds the admitted definition,
not the number of executions. The current once-per-declaration formula is removed because
declarations may be revisited. Replace it with checked admission arithmetic proving a conservative
maximum for the complete sequence and all executions permitted by its recovery bounds, in both
frames and bytes. Include genesis, successful suffixes, domain and eligible execution-fault
attempts, checkpoint value closure, and terminal reports/dispositions. Reject admission before
genesis if the full bound exceeds the format limits. There is no extra durable capacity-stop state
and no requirement that a `Never` State manufacture a domain failure.

Each step's admitted size bounds must cover its complete frame-local closure, including evidence,
context, and classifier/handler/report output; Runtime enforces those bounds on returned values.
Planning may use tighter checked bounds than the global maximum object size, but cannot omit a
possible valid execution within the admitted contract. Exact runtime accounting before every
append checks the admission invariant. Never let a retry reset history accounting.

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

Provide one public reusable default Operation stop report for nonrecoverable incidents, exhausted
allowances, or recovery disallowed by Runtime. It contains the reviewed cause, bounded stop reason,
State position/visit and recovery usage when known, and the mapped typed root domain failure when
the cause is a domain failure. Framework faults retain their structured safe identity and never
require a fabricated root domain failure, including for a State with `Never` domain failure.

Use explicit result variants to distinguish a durably terminal domain or execution failure from an
invocation stopped with runnable, pending, or unknown durable state. An invocation report is not
evidence of an appended terminal conclusion. In particular, pending Effect recovery exhaustion
must report unresolved settlement and retain its exact recovery authority. For failures before a
position is known, report that scope explicitly rather than inventing a State or counter value.
Domain authors may build reporting on these public types; Runtime supplies the default without
requiring a custom reporting Operation.

Preserve stable recovery identities on ambiguous admission/progress acknowledgements. No secrets,
raw provider errors, configuration locators, or arbitrary classifier diagnostics enter public
output. Public reuse does not expose private adapter handles or unredacted source errors.

Application owns mapping into shared client models; CLI and REST update together with their
documented transport asymmetries. They do not implement retry policy or derive run status.

## Complete replacement and deletion scope

The implementation must replace all current producers and consumers together:

1. Program wire, checked construction, and authoring: remove `Declaration::Match`,
   `MatchDeclaration`, `MatchVariant`, `MatchJoin`, `match_join`, `with_failure_handler`, arbitrary
   successor edges, graph joins/frontiers, selector projections, and obsolete graph-only helpers.
   Replace them with sequence construction, scoped checkpoints, public typed classifier/handler
   contracts, and authoring-time resolution of Operation defaults and occurrence overrides.
2. Runtime assembly and engine: delete graph association, Match selection, edge advancement, and
   once-per-declaration assumptions. Implement the sole linear recovery fold, associated classifier
   and handler callbacks, phase-aware public faults, and default reports. Keep one public Runtime
   API and no old engine behind flags or runtime policy-inheritance resolver.
3. Journal and IDs: replace the wire/qualification fixtures and EffectId derivation; update all
   complete-history, hostile-input, capacity, and exact-byte tests, including explicit Read
   execution-fault conclusions and monotonic visits. Store gets no semantic API.
4. Domains and live composition: specialize balance expansion from ordered sources; remove
   `SelectBalanceAsset` and its selector-only sum/metadata/registrations where obsolete. Replace
   Portfolio failure-handler topology with public typed root-failure mapping and resolved
   classification/recovery bindings. Preserve chain/route/input validation and anchored evidence
   contracts.
5. Effect injection and custody consumers: update the four-State transaction sequence, identity
   users, fixtures, and pending recovery tests. Retain exact prepared-wire reuse and signer-free
   recovery guarantees. Do not promote the development finality contract into production.
6. Application, CLI, REST, discovery inventories, examples, and contract snapshots: remove graph
   assumptions and expose the one current run/report model, distinguishing durable termination
   from stopped invocations. Add explicit enrichment-to-admission use cases in the subsequent
   enrichment commit without adding a hidden scheduler.
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
  classifier/handler/failure-mapping contracts, and unbounded allowances fail construction or
  association. Changes to effective policy identity or parameters change Program identity.
- A consuming crate reuses public domain failures, framework/adapter faults, classifiers, handlers,
  and default reports without CLI or private Runtime access. It supplies custom implementations
  for both domain and applicable execution-fault classification/handling.
- Operation defaults, nearest explicit child defaults, and State-occurrence overrides resolve
  deterministically, including injected occurrences. Two occurrences of the same State can choose
  different recovery without modifying the State implementation or adding runtime scope lookup.
- A Read domain failure retries with the same input and new evidence; a Pure deterministic failure
  cannot select same-input retry.
- Read adapter unavailability records a structured execution fault with no fabricated evidence or
  domain outcome. Its retry/restart consumes admitted allowances and exhaustion produces a durable
  execution-failure report; cold fold does not repeat its callbacks.
- Anchor mismatch restarts the declared collection region, discards active suffix values, retains
  prior frames, and yields the same result and head interpretation after cold reload.
- Repeated/nested restart cannot reset local/global budgets; exhaustion produces terminal failure.
- Cancellation and ambiguous append at every recovery boundary converge on one qualified history;
  concurrent callers cannot spend one recovery allowance twice.
- Classifier, handler, mapping, and internal errors append no conclusion or recursively invoke
  recovery. Local binding mismatch performs no provider call or append and cannot be classified
  into authenticated external evidence. Cold fold never calls completed interpreters,
  classifiers, handlers, mappings, providers, or signers.
- Store unavailability cannot falsely spend a durable allowance or report durable termination.
  Read timeout followed by append failure retains the committed budget; ambiguous acknowledgement
  reloads before another decision. Fault assessment preserves origin and phase.
- Pending Effect recovery keeps the exact command/EffectId, cannot jump to a checkpoint, and does
  not rebroadcast a new action after settled success or failure.
- Stopping pending Effect invocation recovery retains its prepare and settlement authority, adds
  no intermediate frame, and reports unresolved execution rather than terminal run failure.
- Restart to an earlier State selects a fresh run-wide visit while extending the latest Journal
  head. Ordinary advancement also selects a fresh visit; interruption without a committed
  transition and repeated pending Effect reconciliation retain the existing visit.
- A checkpoint after an Effect can restart its Read suffix; a checkpoint before it is ineligible,
  including when the Effect was introduced by injection.
- Exact wire/hash vectors, malformed decisions, forged checkpoint refs, stale visits, budget
  overflow, invalid Effect adjacency, and old graph wire are rejected by the correct owner.
- Capacity is checked before Effect authority, with enough reserved space for a maximum admitted
  settlement at the bound; all histories remain within fixed limits.
- Enrichment output linkage is exact; a resumed dependent run does not rediscover assets or depend
  on a deleted config; invalid/stale discovery fails admission under its declared domain policy.
- Default reports distinguish exhausted/nonrecoverable domain failures, durable Read execution
  failures, and stopped invocations with unresolved or unknown history, including unavailable
  State information before association. A `Never` domain failure contract still permits an
  execution-failure report without an encodable domain failure.
- CLI/REST agree on recovery state, reports, and identities while preserving documented transport
  behavior.

## Ordered implementation commits and verification

1. **Record the agreed proposal.** This RFC only; specify public composable classification and
   recovery, default reports, and identity without changing current implementation contracts.
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
- **Resolve Operation policy inheritance during execution:** retains authoring scopes in Runtime
  and multiplies runtime lookup paths. Resolve one effective binding per declaration instead.
- **Classify only domain failures:** excludes ordinary recoverable Read adapter errors. Accept
  structured execution faults while preserving Runtime's phase-specific safety restrictions.
- **Treat all errors as terminal domain failures or generic retries:** fabricates business outcomes,
  loses append uncertainty, and can abandon pending Effects. Keep public typed causes and distinguish
  durable run conclusions from invocation stop reports.
- **Use retry counters as execution identity or fork Journal history on restart:** overlapping
  counters do not replace visit identity or exact-head append. Keep one monotonic VisitId and one
  linear append-only history, independently bounded in frames and bytes.
- **Perform live discovery inside expansion:** makes Program construction depend on ambient IO
  and provides no ordinary durable recovery for that IO. Rejected.
- **Append States to an admitted Program:** changes immutable Program identity and introduces a
  second admission protocol within a run. Separate enrichment/admission is the selected boundary.
- **Implement rollback by truncating history:** loses acknowledged evidence and cannot undo
  external actions. Rejected.

## Material uncertainties

Separate enrichment/dependent admissions, the absence of runtime business branching, hard Effect
barriers, caller-driven progression, authoring-time default resolution, public reusable
error/failure contracts, a shared default report, and budgets counted as committed recovery
decisions are selected design contracts. Remaining uncertainties concern validating the
implementation and concrete enrichment contracts.

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Discovery freezes a bounded asset list with domain-defined validity. | Exhaustiveness and snapshot freshness requirements are not yet specified. | Stronger portfolio claims require anchored discovery and consistency evidence. | Specify expected results when assets or metadata change between discovery and collection. |
| Two public typed implementation contracts with resolved defaults remain compact. | Exact generic Rust signatures for domain failures, execution faults, assessments, and root mappings have not been prototyped. | Excessive type machinery could undermine the authoring simplification or introduce an unwanted domain-to-Runtime dependency. | Implement a minimal consuming-crate prototype with nested Portfolio/EVM defaults, an occurrence override, a custom execution-fault classifier/handler, the default report, and one checkpoint; accept only one compact final API. |
| Conservative full-history admission bounds can fit useful portfolios and recovery allowances. | Per-step complete-closure bounds have not yet been measured against the existing capacity fixtures. | Overly broad bounds could reject useful runs; tighter checked contracts or an explicit format-capacity decision would be required. | Prove bounds for the existing 64-source and transaction fixtures with representative retry budgets before freezing the wire. |
