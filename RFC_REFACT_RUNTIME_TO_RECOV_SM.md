# RFC: replace the graph Runtime with a recoverable linear state machine

Status: proposed. This RFC records the replacement design; it does not change the current
implementation contract in [docs/design.md](docs/design.md) until the implementation cutover.

The [API sketch and engineer handoff](docs/rfc-recovery-api-sketch.md) defines the proposed Rust
types, signatures, exact association, wire shapes, and implementation gates. Its linked standalone
signature proof checks typed composition in the pinned Nix shell; it does not implement Runtime
or prove production codecs, checkpoint scoping, or persistence.

## Decision

Replace the current forward-only State/Match execution model with one immutable, ordered sequence
of typed States and a Runtime-owned recovery state machine.

Configuration determines which States exist before admission. When that information requires IO,
a user-requested enrichment run produces checked values for an immutable configuration revision.
Application uses the selected exact revision to plan the dependent run. Enrichment does not expire
automatically; the user decides when to run it again. Once admitted, a Program never grows or changes.

Normal success advances to the next State. Two independently configurable implementations classify
typed domain failures or adapter errors with State context and choose recovery. Operations supply
defaults; individual State occurrences may override them. Expansion resolves one effective binding
per declaration. Runtime assesses incidents through those bindings, validates requested recovery,
and applies budgets and Effect barriers. Eligible incidents and their recovery decisions are
persisted atomically. Errors whose durable outcome is unknown or cannot be recorded stop the
invocation without inventing a terminal run. Recovery does not require authors to duplicate State
sequences or construct branches.

Unexpected Runtime faults and Store failures stop the affected invocation through a fixed public
error path. They do not enter configurable recovery or manufacture a terminal run conclusion.

This is a complete replacement, not another execution mode. Remove Match, arbitrary success and
failure edges, graph failure handlers, and the old graph compiler and history interpretation.
Preserve the useful boundaries: typed deterministic States, explicit IO capabilities, immutable
content-addressed Programs and values, one Runtime interpretation of committed transitions, exact
append-only Journal history, and mechanical Store IO.

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
than authored as topology, and interpret committed transitions identically during live execution
and reconstruction after restart. It must support bounded retries and restart of a Read region
without erasing history or accidentally repeating external actions.

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
| Program | Immutable ordered State declarations; public incident, execution-phase, classifier/handler authoring contracts; resolved exact bindings, checkpoints, bounded parameters, and typed failure mapping. |
| Domain | Pure State semantics, public typed failures and recovery context for adapter errors, reusable classification/handling implementations, meaningful checkpoints, checked planning values and enrichment meaning. |
| Runtime | Public execution faults and run reports; exact association; authoritative execution phase; sole interpretation of committed transitions; visits, checkpoints, budgets, Effect barriers, and authorized recovery. |
| Capabilities / adapters | Public typed Read observations, Effect settlement, and redaction-safe adapter errors; explicit async IO and duplicate-safe recovery of existing Effects. |
| Journal | Exact canonical frames, object closure, append-only wire qualification, and Effect prepare/conclusion structure. |
| Store | Complete-prefix load and atomic exact-head append only. No recovery, checkpoint, or domain semantics. |
| Application | Validate and bind configuration; persist checked enrichment as an exact immutable configuration revision through the config repository; expose explicit enrichment and dependent-admission use cases. |
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
user requests discovery/enrichment
    -> enrichment Program admission and execution
    -> completed typed discovery output
    -> Application validates and stores an immutable configuration revision
user selects that exact revision
    -> deterministic dependent Operation expansion and capability binding
    -> dependent Program admission and execution
```

Enrichment is ordinary execution with journaled Reads and the same recovery rules. Its output is
a checked typed value. Application applies domain-owned schema, bounds, provenance, and binding
validation, then stores the result through the configuration repository as a named immutable
revision. The DB-backed configuration repository owns that persistence; Runtime does not gain
configuration-write authority. There is no expiry window, scheduled refresh, or rediscovery during
dependent admission or execution. The user explicitly chooses when to run enrichment again and
which resulting revision to use.

Enrichment publishes resolved configuration; it does not assert that observed external facts stay
true indefinitely. Execution retains the domain checks needed for its work, including anchored
Reads and anchor confirmation. Token decimals remain anchored execution Reads unless their domain
contract explicitly makes them frozen configuration. An execution mismatch follows ordinary
failure/recovery handling and never silently replaces configuration or triggers enrichment.

Each admitted stage has its own explicit RunId and immutable Program. The dependent initial
context retains the exact resolved planning value and, when discovery is used, a bounded reference
to the successful enrichment run/head/output. Application verifies that linkage before admission.
That linkage is provenance, not automatic authentication of arbitrary imported provider claims.

Publishing a completed enrichment result creates or compares one exact immutable revision; it
never overwrites a prior revision. Retrying publication uses the same resolved value and revision
identity, without rerunning discovery. This is an explicit Application step after enrichment
completion, not an atomic transaction spanning both runs and configuration storage. Admitted runs
retain everything needed to reconstruct and resume execution and do not require the original
configuration revision to remain in the repository.

The API exposes the stage boundary explicitly. Callers can resume enrichment, read its terminal
output, publish its configuration revision, and request dependent admission with its own stable
RunId. An ambiguous admission is resolved with that same identity and exact input. A matching
admitted run uses its retained inputs rather than re-enriching or requiring a deleted configuration
revision. No hidden automatic cross-run orchestration or crash-recoverable parent workflow is
introduced by this RFC.

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

Each layer adds the meaning it owns while preserving the original typed cause. Adapters expose
reviewed operational errors, such as provider unavailability. States declare typed domain failures
and typed context explaining what an adapter error means for the current work. Runtime supplies
the actual execution phase and durable Effect status; a State or adapter cannot claim that an
unacknowledged append committed or that an unresolved Effect settled. Runtime and Store errors
remain public typed errors with fixed stop behavior, outside configurable classification/handling.

Program owns a small shared incident and execution-phase vocabulary alongside its authoring
contracts. Capability contracts own their public adapter error vocabulary. Runtime constructs the
applicable incident with authoritative execution facts and validates the final action. This keeps
domain policies independent of Runtime without a new fault-introspection framework or crate.

Conceptually, an incident distinguishes `Domain(failure)` from
`Adapter(original_error, state_context)` and is accompanied by Runtime-known execution facts.
State domain-failure returns use the declared typed failure contract. For an adapter error, the
State supplies context through an associated deterministic, IO-free function of its exact input,
prepared intent/command, and typed adapter error. Runtime retains the original adapter error in
the wrapper, so context construction cannot replace it with a business failure or discard its
origin. This context function is part of the State's exact associated implementation contract;
failure of context construction takes the fixed internal-error stop path.

This makes recovery context explicit in the State contract rather than relying on optional
diagnostic strings. States can describe semantic stages such as anchor confirmation or transaction
observation; they cannot override the Runtime's before-prepare, pending, or settled execution facts.
Raw provider errors, arbitrary diagnostics, and secrets never enter incident values, history, or
public output. These are typed extension points, not serialized executable code or an erased bag.

### Two configurable implementations

The names below describe contracts, not a frozen Rust API:

```text
typed domain failure or adapter error with State context
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
exact versioned identities and compatible typed ABIs. They consume only the admitted incident,
parameters, context, execution facts, and counter information. The classifier returns the shared
transient assessment `Recoverable` or `Nonrecoverable`. The handler borrows the same policy-facing
typed incident, that assessment, and admitted recovery context, allowing cause-specific handling
without another intermediate type. Classification does not introduce a persisted assessment
schema or object. Both callbacks perform no ambient IO, read no clock, and receive no Store or
adapter authority. Add richer intermediate categories only for a demonstrated consuming example.

A pending Effect permits only reconciliation of its existing authority. `RetryState` in that phase
means yield for a later explicit reconciliation of the same command, not create a visit or execute
another adapter call in the invocation. `Stop` ends automatic progression with an unresolved-Effect
report. Normal `Pending` observations are progress results, not errors requiring classification.

Typed domain failures map into the Operation's one root domain-failure contract. Adapter incidents
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
unless explicitly configured. A complete binding covers the State's domain failure and applicable
adapter incident, with reusable framework defaults for adapter handling. Runtime and Store errors
do not add policy-association requirements.

Each explicit classifier/handler setting is a finite checked family of exact typed bindings.
Generic implementations such as `Stop<I>` are instantiated for the declared incident input types
during authoring and registration; Runtime cannot instantiate them from a type ID. An explicit
child family replaces that setting's parent family completely. Missing exact coverage is a
construction error, not fallback to an outer family. Only the selected binding is persisted in
each declaration; the families are neither a Program catalog nor runtime inheritance machinery.

### Typed composition examples

These sketches specify the mapping order, with concrete signatures and an executable finite-binding
example in the [API appendix](docs/rfc-recovery-api-sketch.md). Integration with real checked values
and Runtime association remains a consuming-crate gate before the API and wire are frozen.
Each selected classifier declares its input contract. Expansion composes explicit incident mappings
into that contract, using the
existing typed child-to-parent failure mappings and declared State-context mappings. It never
discovers conversions at execution time or adds a second `Operation::Cause` contract.

```text
occurrence-specific classifier:
    ConfirmBalanceAnchor -> EvmBalanceFailure (anchor mismatch)
    -> EVM classifier: recoverable
    -> configured handler: Restart(collection)
    -> Runtime checks checkpoint, mode, and remaining allowance

inherited Portfolio classifier:
    child EVM domain failure
    -> explicit EVM-to-Portfolio failure mapping
    -> Portfolio classifier receives PortfolioSnapshotFailure
    -> configured handler selects recovery or Stop

adapter error contextualized by the State:
    adapter: ProviderUnavailable
    State context: confirming this collection's anchor
    Runtime facts: Read execution; collection checkpoint eligible
    -> selected classifier: recoverable
    -> configured handler: RetryState or Restart(collection)
```

An occurrence-specific classifier sees the child failure before a parent mapping can discard
detail. An inherited parent classifier receives the explicitly mapped parent type. Handler
overrides must accept that policy-facing incident type and the shared assessment. On terminal
domain failure, the preserved typed failure is mapped to the root result; on terminal adapter
failure, the original adapter error and State context remain in the adapter alternative of the
framework report. `Operation::Failure` stays domain-only. This avoids requiring every root domain
to invent an infrastructure failure.

```text
Portfolio default: classify Portfolio incidents; handler Stop; zero allowance
collection child: explicit collection classifier; handler Restart(collection); two restarts
read_balance occurrence: override handler with RetryRead and allow one retry
```

The occurrence inherits the child classifier. Each explicit mapping preserves the original
adapter error while adapting State context to the selected policy scope. The original incident is
retained even if a transient policy-facing mapping reduces detail. The consuming example must also
show a handler-only default working across different State failure types without adding an erased
error registry or parallel domain/adapter authoring APIs.

### Runtime authorization and error boundaries

Runtime uses the configured classifier and handler for domain failures and eligible contextualized
adapter errors, and retains sole authority over legal transitions. A policy cannot bypass mode
restrictions, restore arbitrary values, reset counters, or authorize another Effect. Structurally
invalid policy results and classifier/handler failures return an internal error without recursively
entering the same recovery pipeline. Expected exhaustion or an ineligible checkpoint produces the
standard stop report; it becomes terminal only where a valid durable conclusion is possible.

| Incident | Authorized behavior |
| --- | --- |
| Typed domain failure | Classify and atomically conclude with mode-eligible retry/restart or terminal domain failure. |
| Read adapter unavailability without accepted evidence | Preserve its typed cause, add State context, classify, and atomically record retry, eligible restart, or terminal execution failure. Never fabricate evidence or a domain outcome. |
| Pending Effect operational adapter error | Contextualize and assess while retaining the exact prepare, visit, command, and EffectId. Yield for same-authority reconciliation or stop automatic progression with an unresolved report; never fabricate settlement. |
| Store failure, including an uncertain append acknowledgement | Stop the affected invocation immediately through its public typed error; no classifier, handler, further append, or automatic Store retry. Preserve RunId and acknowledgement uncertainty for a later explicit history load. |
| A competing append was accepted at the expected head | Load the winning complete history and return its current view without executing a newly authorized retry. A load failure takes the fixed Store-error stop path. |
| Preparation, interpretation, State error-context construction, local binding, adapter invariant, or Runtime invariant error | Stop through the public typed internal-error path, without configurable recovery, a fabricated domain conclusion, new provider call, or recovery append. |
| Invalid history, incompatible association, or error before a trustworthy execution position exists | Stop through the public framework report; do not invoke an unassociated policy or execute against invalid history. |
| Cancellation | Preserve committed authority; no guaranteed callback or append during cancellation. A later caller reloads and resumes. |

For an internal Pure/Read callback error, the current durable head remains unchanged. For an Effect
callback error after preparation, its exact acknowledged prepare remains pending. Failure or
uncertainty while appending an incident decision stops the invocation; the append error does not
reenter the classifier/handler pipeline. No automatic attempts follow an internal or Store failure.

Stopping the application means stopping the affected invocation: the CLI exits with its redacted
error; a service reports the failed request without terminating unrelated runs. Neither boundary
claims that an unrecorded stop is a durably terminal run. Recovering uncertain storage progress
requires an explicit load/read with the original identity before another resume.

## Checked planning and exact input specialization

The concrete sequence is specialized to its checked root input. `Operation::validate_input` is a
required deterministic authoring check of the Operation's planning assumptions against that input.
`expand_program` performs it before expansion and commits the qualified initial-value content ref
into Program identity. This check establishes plan/input agreement; the content ref separately
prevents later substitution. Runtime compares supplied input before genesis or provider entry and
checks retained genesis against it during reconstruction. Multiple RunIds can reuse the same exact
Program/input pair. Parent planning and deterministic States establish future child input contracts;
Runtime receives no domain validation callback.

For balance collection, the checked ordered request determines native/token State sequences during
authoring. Remove selector-only States and Match payloads. The initial-anchor State updates the
ordinary context stage using the checked current source; it does not select execution topology.

## Checkpoints and restart semantics

A checkpoint names the input boundary before a State. Its active snapshot is the exact immutable
input at the most recent entry to that checkpoint on the current execution path. Genesis provides
the initial boundary. A checkpoint references retained qualified values; it is not a mutable Store
snapshot or a second persistence service.

The current activation and its exact input are derived from the preceding committed history.
A restart decision records only its declared checkpoint position; Runtime selects that position's
current eligible activation and retained input. Callers and handlers cannot supply an alternative
activation or restored value. Merely invoking `resume` cannot create another activation. The
committed restart establishes its replacement without an extra checkpoint frame or independently
persisted activation identity.

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

Runtime reconstructs the current execution position from committed history. For example, records
of A succeeding and B failing with an authorized retry reconstruct a runnable B with one recovery
decision spent; reconstruction does not execute A, B, the classifier, or the handler again. The
same transition interpretation updates the position immediately after a successful append.

Its active state distinguishes a runnable State visit, an exact pending Effect, and terminal
success/failure. It also derives active checkpoint inputs and monotonic recovery counters. Avoid
correlated status flags, independent mutable cursors, and a second history interpreter.

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
Each normal pending Effect observation yields rather than retrying internally. There is no durable
pending-check retry counter or automatic settlement retry loop. If handling an adapter error stops
recovery with an unresolved Effect, the invoking application stops automatic progression and
reports "Recovery stopped; transaction X remains unresolved" (or the corresponding nontransaction
Effect description). It must not interpret this report as a request to retry. Any later
reconciliation requires an explicit caller action using the retained identity; it cannot authorize
a replacement action. No new durable stopped-Effect status is introduced. Runtime makes no
eventual-settlement guarantee.
Exhausted recovery allowances cannot revoke an acknowledged pending command: its retained visit
must remain eligible for explicit settlement and its admitted conclusion capacity remains available.

## Journal, atomicity, and history reconstruction

Introduce one current linear-recovery Program and Journal wire contract with new identities. Old
graph bytes are rejected. Do not reinterpret them as sequences or add compatibility readers.

Pure/Read domain conclusions retain the typed outcome; Read domain conclusions also retain exact
intent and accepted evidence. A distinct Read execution-fault conclusion retains the exact intent,
original reviewed typed adapter error, State context, and recovery disposition, with no accepted
evidence or domain outcome. Origin and applicable phase must agree with the associated declaration
and execution history. These alternatives must be represented explicitly, not through a fabricated evidence
value or unrelated optional fields. An incident conclusion retains the authorized disposition,
target checkpoint position for restart, stop reason when terminal, and the mapped root domain
failure where applicable. Runtime derives checkpoint activation/input, position, visit, and
recovery usage from the qualified history rather than persisting an independently assembled copy
of those facts in a report. Retained terminal facts suffice to construct and content-address the
one canonical public output, which remains subject to the canonical-object size bound; no reporting
callback runs during reconstruction.
Effect prepare retains the command, visit, and EffectId; its adjacent conclusion retains bound
settlement and the terminal-or-advance disposition.

An eligible incident and recovery decision are one atomic conclusion. There is no durable
intermediate failed State waiting for an unrecorded classifier or handler choice. If classification,
handling, mapping, or validation fails before append, no conclusion is committed. An Effect's
prepare remains pending in that case. Pending Effect errors add no intermediate frames between
prepare and settlement conclusion; stop reports for those invocations are not settlement records.

Journal owns structural wire qualification and complete frame-local object closure. Runtime
reconstructs execution against the associated Program, checking expected visit and position,
allowed mode, checkpoint eligibility and input, counters, fault eligibility, result contracts, and
Effect barriers. Completed policy decisions are authoritative recorded results, like completed
State outcomes. Reconstruction checks structural validity and Runtime safety constraints; it does
not prove that a classifier or handler would return the recorded decision by executing it again.
Arbitrary rewritten histories do not become authenticated merely by passing those checks.

Reconstruction never reruns completed classifiers, handlers, State error-context construction,
failure mappings, or State interpretations and performs no provider or signer IO. Only the final
unresolved Effect is re-prepared deterministically to validate its exact command and identity
before reconciliation. Completed Effect prepares retain structural, contract, identity, and
settlement-binding checks without executing their preparation callbacks again. This is a deliberate
change from the current engine, which invokes preparation validation for every retained prepare.

Store remains mechanical. Every append is exact-head and all-or-nothing. A losing concurrent
append reloads the winning complete history. For example, two callers may read the same head and
propose recovery; Store accepts one decision and rejects the other's outdated append. After loading
the winner, the losing invocation returns the reconstructed current view without executing a newly
selected recovery visit. Callers resolve an uncertain append through `read` with the original RunId
before explicitly requesting further progression. Concurrent recoveries cannot each spend the same
budget or establish competing active snapshots. Concurrent Effect invocations must remain safe
through the adapter's same-identity protocol; Store locking alone does not prevent duplicate IO.

### Capacity

Keep an independent finite Program declaration/size bound and explicit finite canonical-object,
frame, frame-count, and total-history byte limits. Declaration count bounds the admitted definition,
not the number of executions. The current once-per-declaration formula is removed because
declarations may be revisited. Replace it with checked admission arithmetic proving a conservative
maximum for the complete sequence and all executions permitted by its recovery bounds, in both
frames and bytes. Include genesis, successful suffixes, domain and eligible execution-fault
attempts, checkpoint value closure, and retained terminal facts/dispositions. Reject admission before
genesis if the full bound exceeds the format limits. There is no extra durable capacity-stop state
and no requirement that a `Never` State manufacture a domain failure.

Start with one conservative checked rule, calculated separately for frames and bytes:
`genesis_max + (global_recovery_allowance + 1) * sequence_max`. The allowance counts committed
retry/restart decisions. `sequence_max` sums each declaration's maximum complete lifecycle,
including success, eligible failed conclusion, or Effect prepare plus conclusion as applicable.
Between recovery decisions execution advances monotonically, so each segment costs at most that
sum; the final segment includes termination. Use the maximum across alternative conclusions, not
only the success size. Validate this rule against useful portfolios and transaction fixtures before
freezing the format; introduce tighter analysis only if those measurements demonstrate a need.

Each step's effective size bounds are immutable Program data and participate in its content
identity. They cover complete frame-local closure, including evidence, context, State-supplied
error context, and retained terminal facts. Transient classifier assessments and constructed
reports do not introduce duplicate frame objects. Runtime enforces bounds on returned values and
checks association against the admitted contract.
Planning may use tighter checked bounds than the global maximum object size, but cannot omit a
possible valid execution within the admitted contract. Exact runtime accounting before every
append checks the admission invariant. Never let a retry reset history accounting.

Before preparing an Effect, Runtime checks sufficient remaining frame/count/byte capacity for both
its prepare and its maximum admitted conclusion, including terminal failure facts. The required
capacity is derived from that declaration's immutable Program bounds and actual history usage,
including when reconstructing an unresolved Effect. There is no separate persisted reservation
field or capacity ledger. Evidence larger than the capability's admitted bound is rejected. No
adapter is entered for an Effect whose complete bounded durable lifecycle cannot fit.

For Pure/Read execution, check available conclusion capacity before IO where IO applies. A retry
or restart must not authorize a visit that has no capacity for its required bounded conclusion.
Capacity admission and error behavior must be covered at the limit, including a pending Effect
near the history bound. Do not claim that this storage guarantee ensures external settlement.

## Public behavior

Keep one typed start/resume/read surface. `read` reconstructs the execution position from committed
history without executing States. `resume` progresses a runnable visit or explicitly reconciles
the exact pending Effect; terminal resume is unchanged and performs no execution. Runnable
recovery exposes a bounded reason and target State position so callers can distinguish retry/restart
from pending settlement without inspecting frames.

Provide one public reusable default run stop report for nonrecoverable incidents, exhausted
allowances, or recovery disallowed by Runtime. It contains the reviewed cause, bounded stop reason,
State position/visit and recovery usage when known, and the mapped typed root domain failure when
the cause is a domain failure. Framework faults retain their structured safe identity and never
require a fabricated root domain failure, including for a State with `Never` domain failure.

Use explicit result variants to distinguish a durably terminal domain or execution failure from an
invocation stopped with runnable, pending, or unknown durable state. An invocation report is not
evidence of an appended terminal conclusion. In particular, stopped recovery for a pending Effect
reports unresolved settlement and retains its exact recovery authority, with no automatic
continuation or claim of a spent durable pending-check allowance. For failures before a
position is known, report that scope explicitly rather than inventing a State or counter value.
Domain authors may build reporting on these public types. Domain owns terminal failure mapping,
Runtime owns deterministic report construction from retained facts, and Application owns transport
rendering. There is no custom reporting Operation, synthetic reporting State, or separately stored
report duplicating the original incident and derived metadata.

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
   once-per-declaration assumptions. Implement one interpretation of linear recovery transitions,
   associated classifier and handler callbacks, typed incident/context construction, fixed
   Runtime/Store stop paths, and derived default reports. Keep one public Runtime
   API and no old engine behind flags or runtime policy-inheritance resolver.
3. Journal and IDs: replace the wire/qualification fixtures and EffectId derivation; update all
   complete-history, hostile-input, capacity, and exact-byte tests, including explicit Read
   execution-fault conclusions and monotonic visits. Derive checkpoint activation, capacity
   allowance, and report metadata without duplicate wire fields. Store gets no semantic API.
4. Domains and live composition: specialize balance expansion from ordered sources; remove
   `SelectBalanceAsset` and its selector-only sum/metadata/registrations where obsolete. Replace
   Portfolio failure-handler topology with public typed root-failure mapping and resolved
   classification/recovery bindings. Preserve chain/route/input validation and anchored evidence
   contracts. Delete the obsolete mapping State's registration and inspection metadata when its
   pure mapping becomes the associated domain-failure conversion.
5. Effect injection and custody consumers: update the four-State transaction sequence, identity
   users, fixtures, and pending recovery tests. Retain exact prepared-wire reuse and signer-free
   recovery guarantees. Do not promote the development finality contract into production.
6. Application, CLI, REST, discovery inventories, examples, and contract snapshots: remove graph
   assumptions and expose the one current run/report model, distinguishing durable termination
   from stopped invocations. Add explicit enrichment publication to immutable DB configuration
   revisions and dependent-admission use cases in the subsequent enrichment commit, with no hidden
   scheduler, expiry window, or automatic rediscovery.
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
  for domain and contextualized adapter incidents while Runtime and Store failures bypass policy.
- The consuming examples cover original child failure versus mapped parent classification,
  handler-only defaults across different failure types, and independent occurrence overrides.
  Incompatible policy-facing input contracts are rejected before execution.
- State context construction preserves the original adapter error and cannot replace Runtime's
  actual execution phase. Anchor confirmation and pending transaction examples exercise typed
  context, cause-specific handling, safe reporting, and context-construction failure.
- Operation defaults, nearest explicit child defaults, and State-occurrence overrides resolve
  deterministically, including injected occurrences. Two occurrences of the same State can choose
  different recovery without modifying the State implementation or adding runtime scope lookup.
- A Read domain failure retries with the same input and new evidence; a Pure deterministic failure
  cannot select same-input retry.
- Read adapter unavailability records a structured execution fault with no fabricated evidence or
  domain outcome. Its retry/restart consumes admitted allowances and exhaustion produces a durable
  execution-failure report; history reconstruction does not repeat its callbacks or State context
  construction. Transient policy mappings do not replace the original retained incident.
- Anchor mismatch restarts the declared collection region, discards active suffix values, retains
  prior frames, and yields the same result and execution position after reconstructing history.
  Restart records only its checkpoint position and selects the current eligible input from history.
- Repeated/nested restart cannot reset local/global budgets; exhaustion produces terminal failure.
- Cancellation and ambiguous append at every recovery boundary converge on one qualified history;
  concurrent callers cannot spend one recovery allowance twice. A losing caller loads and returns
  the winning position without executing the newly selected recovery visit in that invocation.
- Classifier, handler, mapping, and internal errors append no conclusion or recursively invoke
  recovery. Local binding mismatch performs no provider call or append and cannot be classified
  into authenticated external evidence. History reconstruction never calls completed interpreters,
  classifiers, handlers, mappings, State context functions, providers, or signers. It accepts
  recorded policy decisions as authoritative while rejecting violations of Runtime safety rules.
- Completed Effect preparation callbacks are not rerun during reconstruction; the final unresolved
  Effect is re-prepared for exact command/identity validation without provider or signer IO.
- Runtime and Store failures stop the affected invocation without configured classification,
  further IO attempts, a false durable allowance charge, or claimed terminal conclusion. A failed
  recovery append does not reenter policy. Uncertain acknowledgement retains its RunId and unknown
  commitment; an explicit history read resolves it before further requested progress.
- Pending Effect recovery keeps the exact command/EffectId, cannot jump to a checkpoint, and does
  not rebroadcast a new action after settled success or failure.
- Stopping pending Effect invocation recovery retains its prepare and settlement authority, adds
  no intermediate frame or durable attempt count, and reports unresolved execution rather than
  terminal run failure. Application stops automatic progression; later reconciliation requires
  explicit caller action. Normal `Pending` still permits ordinary explicit resume.
- Restart to an earlier State selects a fresh run-wide visit while extending the latest Journal
  head. Ordinary advancement also selects a fresh visit; interruption without a committed
  transition and repeated pending Effect reconciliation retain the existing visit.
- A checkpoint after an Effect can restart its Read suffix; a checkpoint before it is ineligible,
  including when the Effect was introduced by injection.
- Exact wire/hash vectors, malformed decisions, undeclared/ineligible checkpoint targets, stale
  visits, budget overflow, invalid Effect adjacency, and old graph wire are rejected by the correct owner.
- Capacity is checked before Effect authority, with enough space for a maximum admitted settlement
  at the bound; all histories remain within fixed limits. Changing effective size bounds changes
  Program identity; reconstruction derives capacity from Program and history without a separate
  persisted reservation. Exercise the conservative formula on the 64-source and transaction fixtures.
- Enrichment output linkage is exact; a resumed dependent run does not rediscover assets or depend
  on a deleted config. Invalid schema, provenance, or binding is rejected; elapsed time alone never
  expires enrichment or triggers rediscovery. Explicit enrichment creates a new immutable revision
  and leaves prior revisions and admitted runs unchanged. Repeating publication after a lost
  acknowledgement compares the same revision without another discovery run. Recovering an existing
  dependent admission uses its retained inputs.
- Default reports distinguish exhausted/nonrecoverable domain failures, durable Read execution
  failures, and stopped invocations with unresolved or unknown history, including unavailable
  State information before association. A `Never` domain failure contract still permits an
  execution-failure report without an encodable domain failure. Reports derive metadata and their
  canonical output reference from retained facts without a separately stored duplicate report.
- CLI/REST agree on recovery state, reports, and identities while preserving documented transport
  behavior.

## Ordered implementation commits and verification

1. **Record the agreed proposal.** Documentation only; specify public composable classification and
   recovery, the linked type/signature appendix and standalone proof, derived data, fixed stop
   behavior, and explicit enrichment revisions without changing current implementation contracts.
   Before implementation freeze, integrate the consuming API with real codecs and validate capacity
   arithmetic; the standalone proof does not replace those gates.
2. **Replace the execution contract completely.** Keep the inseparable Program/Runtime/Journal/
   Effect-identity cutover, all dependent domain/adapter/application/binary changes, deletions,
   fixtures, tests, and current design documentation in one coherent logical commit. Do not split
   it by crate if doing so requires an intermediate dual design or broken consumer.
3. **Add live enrichment workflows.** Once the linear contract exists, add concrete discovery
   Operations, publication to immutable DB configuration revisions, and explicit dependent-admission
   use cases with checked provenance/binding and idempotent publication tests and docs. Select the
   concrete discovery source and value schema for that follow-up. Static configured portfolios
   already work after the replacement commit; no freshness expiry or automatic discovery is added.

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
  original typed adapter errors with State context while preserving Runtime-owned execution facts.
- **Configure recovery of Runtime or Store failures:** adds policy paths where trusted execution
  or persistence cannot continue. Stop the invocation through public typed errors without retrying
  the failed recovery append or terminating unrelated service work.
- **Persist derived checkpoint activations, capacity reservations, or report metadata:** duplicates
  authority already in Program and committed history. Derive these values in Runtime instead.
- **Reevaluate recorded policy decisions when loading history:** repeats decisions that have already
  committed. Reconstruct the position from recorded results and validate Runtime safety constraints.
- **Expire enriched configuration or rediscover automatically:** adds an unrequested configuration
  lifecycle. Retain immutable revisions and let the user decide when to enrich and select a revision.
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
barriers, caller-driven progression, authoring-time default resolution, layered public typed
incidents, fixed Runtime/Store invocation stops, derived reports, authoritative recorded decisions,
and budgets counted as committed recovery decisions are selected design contracts. Enrichment has
no freshness window; users publish and select immutable revisions explicitly. Remaining
uncertainties concern implementation validation and the concrete discovery follow-up.

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Finite exact typed bindings integrate compactly with real checked values. | The standalone Rust proof validates generic selection and non-Clone mapping, but substitutes marker values and TypeId for real codecs/content refs. | Codec/association bounds could require revising the signatures. | Port the appendix's four consuming cases to real MfmValue types and Runtime assembly, including original error preservation and typed report decoding. |
| Typed checkpoint tokens with checked scope identity are sufficient. | The signature proof does not implement scope bookkeeping or injection. | Incorrect construction checks could permit direct parent-token capture. | Test legal parent-installed inherited recovery and rejected direct child reinstallation after complete injection. |
| Input plus prepared intent/command provides sufficient State recovery context. | Concrete adapter-error augmentation has not been exercised. | Additional retained facts could enlarge the State callback contract. | Exercise anchor confirmation and pending transaction errors; add only inputs required by those examples. |
| A bounded discovery output can publish through the existing configuration repository contract. | The concrete discovery source and output schema are not selected. | The enrichment follow-up may need a domain-specific schema or publication binding. | Choose one discovery workflow and test exact revision publication, repeated publication, and dependent admission without expiration or rediscovery. |
| Conservative full-history admission bounds can fit useful portfolios and recovery allowances. | Per-step complete-closure bounds have not yet been measured against the existing capacity fixtures. | Overly broad bounds could reject useful runs; tighter checked contracts or an explicit format-capacity decision would be required. | Prove bounds for the existing 64-source and transaction fixtures with representative retry budgets before freezing the wire. |
