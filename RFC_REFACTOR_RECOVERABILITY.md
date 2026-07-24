# RFC: Refactor Projection, Recoverability, and Saga / AC/DC

Status: proposed

Scope: typed runtime, certified specs, events, store, replay, app status, and mutation adapters

This RFC proposes a breaking replacement of the current projection, side-effect recovery, and
generic saga machinery. It is not the current authoritative contract. If accepted, the
[design contract](docs/design.md), [architecture guide](docs/architecture.md), affected companion
documents, code, schemas, and tests must move to this design together.

## Executive Decision

MFM remains a platform for typed on-chain writes whose mutation protocol satisfies the certified
identity, observation, and recovery contract. `ApplySideEffect` and interruption recovery remain
core platform primitives.

The proposed refactor is:

1. Replace the current multi-phase side-effect ledger with one durable, node-scoped effect journal.
2. Make one committed `EffectArmed` record the only boundary after which an external mutation call
   is authorized.
3. Recover an armed effect by observing its stable external identity or, when explicitly certified,
   repeating the exact same invocation.
4. Keep worker attempts and interruption, but make them operational driving history rather than
   mutation identity or external truth.
5. Replace generic saga, compensation, manual-resolution, and AC/DC outcomes with explicitly
   certified follow-up operations containing ordinary `ApplySideEffect` nodes.
6. Replace the public, universal projection snapshot with one store-owned structural per-run fold,
   one opaque runtime verification wrapper over that fold, and narrowly named coordination
   structures.

The target is not a read-only core and is not a weaker form of “try the write again.” It is a
smaller write protocol with a precise uncertainty boundary and guarantees MFM can substantiate
under its certified spec, append-only stream, retained evidence, and recorded assurance contracts.

## Problem Situation

### The platform contract and the current catalog have been conflated

The current production catalog is dominated by pure and external-read states. That is an
implementation snapshot, not the intended scope of MFM. Transfers, deployments, contract calls,
approvals, staking, bridging, governance, and other mutations are target use cases. Each concrete
protocol must still qualify against the identity, observation, recovery, and serialization
requirements in this RFC.

Removing `ApplySideEffect` would postpone the hardest platform contract and force each future write
workflow to invent recovery independently. The correct simplification is to reduce the mutation
kernel, not remove it.

### Projection became a second model of the runtime

The current projection surface collects admission, attempts, cells, side-effect phases, saga
state, resource claims, retention, public output, and terminal state into a broadly shared snapshot.
That creates several problems:

- a change to one lifecycle crosses store, runtime, replay, app, and transport-facing status code;
- callers can accidentally treat derived maps as independent semantic authority;
- independently loaded projections can represent different stream watermarks;
- store validation and runtime scheduling can implement similar folds differently;
- protocol-specific mutation phases become generic projection fields;
- a general “projection” abstraction obscures the difference between a verified per-run view, a
  query index, and mutable operational coordination.

Projection is useful as a technique. The problem is a public universal projection model that
duplicates the append-only stream and becomes a future change site for every concern.

### Recoverability is distributed across too many identities and phases

The current mutation lifecycle distinguishes intent, claim, prepared invocation, invocation
started, submission known or unknown, receipt, confirmation, ambiguity, failure, attempt recovery,
and saga recovery. These distinctions are individually understandable, but together they create a
large phase cross-product.

The resulting costs are:

- attempt identity, side-effect ledger identity, pair identity, claim ownership, and external
  operation identity can drift apart;
- interruption classification must understand every mutation phase;
- protocol-specific receipt and confirmation concepts leak into the kernel;
- “submission unknown” duplicates the fundamental fact already established by crossing the durable
  mutation boundary;
- a process crash can change which recovery branch is selected even though it cannot change what
  happened externally;
- signed bearer material cannot be stored under MFM's security rules, while parts of the recovery
  design behave as though a prepared invocation is always durably recoverable;
- fixing one recovery edge requires coordinated edits across events, store typestate, projections,
  scheduler decisions, replay, adapters, APIs, and tests.

The missing distinction is between:

- the logical external effect;
- an exact external invocation for that effect; and
- a worker attempt that temporarily drives the effect.

Those are different identities and must remain different.

### Generic saga and AC/DC semantics claim more than the kernel can prove

The current [certified saga contract](docs/saga.md) honestly narrows AC/DC, but it still requires the
kernel to understand forward and remediation roles, pair linkage, engagement, quiescence,
obligations, reverse remediation order, run modes, terminal proof, manual authorization, resource
claim classes, and public compensation outcomes.

That machinery has two structural limitations:

1. A generic kernel cannot prove that an arbitrary compensation restores business equivalence in
   an external system.
2. A signed operator decision proves authorization to make a decision. It does not prove external
   domain truth.

Compensation is still necessary. It is also domain behavior. A token refund, bridge recovery,
replacement transaction, revocation, or governance correction has different preconditions,
concurrency rules, evidence, and meaning. Encoding them as generic saga policy moves domain
semantics into the kernel while still stopping short of an honest cross-system atomicity claim.

### The carrying cost now exceeds the delivered guarantee

The current design has accumulated:

- many public event and policy types;
- duplicated owned and borrowed lifecycle representations;
- broad projection and status DTOs;
- generic resource-lane and waiter protocols;
- manual-resolution proof infrastructure;
- saga-specific scheduler branches and terminal modes;
- protocol-specific recovery concepts in domain-free crates.

This makes every evolution expensive without providing external exactly-once execution,
cross-system atomicity, rollback, or general compensation equivalence. The right response is a
complete cutover to a narrower guarantee, not another compatibility layer over the existing one.

## Goals

- Keep writes as a first-class certified effect.
- Preserve interruption-safe execution across process death and worker takeover.
- Make the durable mutation uncertainty boundary explicit and singular.
- Give every mutation one stable logical identity independent of attempts.
- Prevent recovery from preparing a semantically different invocation after `EffectArmed` commits.
- Require replay-verifiable domain evidence for every terminal mutation classification.
- Keep state logic pure and live IO inside adapters and explicit capabilities.
- Preserve append-only history, atomic commits, content addressing, canonical JSON, and no-secret
  persisted surfaces.
- Replace generic compensation with explicit typed follow-up workflow semantics.
- Make run status, scheduling, resume, and replay consume the same committed stream watermark.
- Minimize concepts, public types, code paths, duplicated responsibilities, future change sites,
  and lines of code.

## Non-Goals

- Exactly one physical RPC request.
- Exactly-once behavior from arbitrary external systems.
- Cross-chain or cross-system atomicity.
- Generic rollback or restoration of prior business state.
- A generic proof that compensation is equivalent to the original state.
- Concurrency control over wallets, validators, operators, or applications outside MFM.
- Perpetual finality after the certified domain finality condition has been satisfied.
- Generic transaction replacement, fee bumping, nonce management, or UTXO selection policy.
- Parallel open node attempts within one run.
- Automatic or guaranteed admission of a corrective follow-up run after its source run settles.
- A universal manual override that can declare an unknown mutation successful or absent.
- Backward compatibility with current event, store, status, or saga schemas.

## Material Uncertainties

| Choice or assumption | Why it is uncertain | Consequence if wrong | Resolution or validation |
| --- | --- | --- | --- |
| Signed bearer invocations remain outside ordinary events and artifacts. `RepeatExact` therefore requires deterministic reconstruction or an approved secure provider. | Some chains require exact signed bytes after a crash, while MFM forbids persisting raw signed transactions and secrets. | An adapter may be limited to `ObserveOnly` and an effect can remain unresolved after a crash between arming and submission. | Threat-model each write adapter. Enable `RepeatExact` only after review and conformance tests justify its exact reconstruction, non-secret durable references, redaction, and no bearer leakage assumptions. |
| The EVM adapter does not yet have a selected non-bearer `InvocationWitness` contract. | A transaction hash alone may not prove every required intent, signer, network, policy, external-identity, and exact-invocation binding without retaining bearer bytes. | The current `SubmitEvmTransactionState` cannot be certified or registered under this RFC even in `ObserveOnly` mode. | Design and review a concrete domain-separated signer or provider attestation, including tamper, replay, redaction, and pure-verification tests. Keep EVM mutation registration disabled until it passes the adapter qualification suite. |
| `RepeatExact` assumes overlapping delivery of the exact invocation cannot create another mutation or separately meaningful cost-bearing effect. | The original worker, a stale recovery worker, and the current worker can race despite database fencing. Some providers may charge or act per delivery rather than per operation identity. | Enabling exact repeat could duplicate external cost or behavior even when bytes and operation id match. | Document the adapter's deduplication boundary and run concurrent/delayed conformance tests. Use `ObserveOnly` when the assumption is not justified. |
| Every supported write adapter can derive a stable external operation identity before dispatch. | Some systems assign an identity only after accepting a request. | Such a system cannot support safe generic interruption recovery from `EffectArmed`; observation cannot target the exact operation. | Inventory intended chains and protocols. Require a client-derived identity, external idempotency service, or a separate future protocol before certifying the adapter. |
| `VerifiedNotApplied` requires positive evidence that the exact recorded invocation did not apply and can no longer apply under the certified assurance model. | Lookup absence, timeout, dropped mempool state, and provider disagreement usually establish neither condition. | Runs and serialization keys may remain blocked indefinitely. A weaker rule could allow a stale worker to apply an invocation after MFM released its fence. | Define and test a domain verifier for each supported non-application proof. If none exists, the adapter must omit this resolution. |
| `active_effect_keys` relies on a trusted store boundary rather than detecting arbitrary database tampering on every arm. | A database administrator, compromised role, or storage fault can bypass normal store transactions and remove or alter a constraint row after startup verification. | An undetected out-of-band change could defeat MFM-local serialization even though the event journals remain authoritative. | Give only the MFM store role mutation access, verify or rebuild the table under writer exclusion before enabling arms, fail closed when an integrity audit detects divergence, and treat direct SQL tampering or storage corruption as outside the certified execution guarantee. |
| One unresolved effect per run and at most one serialization key per effect are sufficient initially. | Independent writes could be parallelized, and some operations touch multiple resources. | The design may over-serialize high-throughput workflows or require a deliberately coarse key. | Validate against the first concrete multi-write operations. Add concurrency only with a new proof and deadlock model; do not prebuild generic multi-key locking. |
| Pre-arm preparation is safe without a durable resource reservation. | Some domains select a nonce, UTXO set, or other ordering input during preparation. | Concurrent preparation can waste signing work or create an invocation that becomes stale before arming. | Require adapters to tolerate discarded preparation, destroy every failed candidate, and verify a freshly prepared identity before dispatch. An adapter needing pre-preparation exclusivity is not certifiable under this RFC and requires a future `EffectDeclared`-style protocol. |
| An unresolved or disputed effect is allowed to block forever rather than accept an operator assertion as truth. | Operational teams may require a break-glass path. | A run and its serialization key can remain nonterminal indefinitely. | Initially allow operators only to trigger certified observation. Specify any future authenticated proof ingress or abandonment action as its own domain contract; abandonment makes a risk-acceptance claim, not an external truth claim. |
| In-place fee bumping or replacement is excluded. | Some chains commonly require replacement while the original invocation may still be executable. | A transaction can remain unresolved or economically stuck. | Allow a fresh invocation only after `VerifiedNotApplied`, or design a separate adapter-specific multi-invocation equivalence protocol before supporting a live replacement. |
| A domain finality verifier may terminalize an effect even though a later reorganization is theoretically possible. | Finality strength differs by chain and policy. | A later observation can contradict a resolution that was valid under the recorded assumptions, but append-only history cannot be rewritten. | Make the evidence provenance, finality policy, verifier identity, and assurance summary hash-defining. Represent later invalidation only through a new linked operation or run; release of a serialization key remains conditional on the recorded assurance assumption. |
| Existing persisted histories may be rejected during the cutover. | MFM is pre-production, but an environment may still contain data worth inspecting or exporting. | Resetting the event and Postgres baseline makes old runs unreadable by the new binary. | Inventory deployments before implementation. Export required evidence first, then reject old schemas explicitly; do not add legacy readers or dual writers. |

## Terminology

**Effect**
: One logical external mutation owned by one certified `ApplySideEffect` node.

**Effect identity**
: The stable MFM identity for that logical effect. It is derived from canonical
  `(run_id, node_id)` material and does not contain an attempt id.

**Invocation**
: The exact protocol request that may cause the effect, such as one signed transaction envelope.

**External operation identity**
: A stable domain identity used to observe the exact invocation, such as a transaction hash.

**Attempt**
: A bounded period during which one worker drives a node. Attempts can be interrupted and replaced;
  effects cannot.

**Observation**
: Typed evidence about the recorded external operation identity. Observation is not mutation.

**Resolution**
: The one terminal, verifier-accepted classification of an effect.

**Serialization key**
: An optional key deterministically derived by domain/state semantics from certified intent and
  typed inputs. It prevents two MFM effects with the same key from being armed concurrently. It
  does not fence external actors.

**Compensation**
: A new, explicit domain mutation intended to correct or offset an earlier resolved mutation. It is
  not rollback and is not a kernel-inferred obligation.

## Proposed Solution

### Target runtime shape

```text
certified typed graph
        +
committed run stream at one head
        +
verified retained evidence
        |
        v
runtime VerifiedRunView over store RunJournalFold
        |
        +--> deterministic frontier decision
        |
        +--> start or resume one worker attempt
                  |
                  +--> pure/read settlement
                  |
                  `--> one durable effect journal
                           |
                           +--> arm before mutation
                           +--> observe or repeat exact
                           `--> resolve and settle atomically
```

The certified typed graph remains the semantic program. The run stream remains append-only
authority. Adapters remain the only owners of live mutation IO. Replay remains evidence-only.

### One effect journal per `ApplySideEffect` node

Every `ApplySideEffect` node has one stable `EffectId`:

```text
EffectId = content_address(
  canonical_json({
    "contract": "mfm.effect-id.v1",
    "run_id": run_id,
    "node_id": node_id
  })
)
```

The exact namespace and version are part of the final schema decision. The important invariant is
that an attempt id, worker id, process id, lease token, retry number, or provider response cannot
change the effect identity.

A run may contain many sequentially resolved effects. It may have at most one unresolved effect.
Certified graphs remain acyclic and each node settles at most once. Repeating a business action
requires a distinct certified node or a distinct run. Separate runs are not deduplicated merely
because their human-readable intent describes an equivalent action; any cross-run request
deduplication must come from the existing run identity contract or an explicit domain protocol.

### Minimal effect algebra

```text
Unarmed
   |
   | EffectArmed
   v
Armed ------------------------------> Disputed
   |       positive conflicting          |
   |       identity or evidence           |
   |                                      |
   +---------- verifier-accepted ---------+
                  terminal evidence
                         |
                         v
                      Resolved
```

`Resolved` has one core classification and an assurance summary:

- `VerifiedAppliedSuccess`: evidence accepted under the certified assurance model says the external
  mutation applied and satisfied its success condition;
- `VerifiedAppliedFailure`: evidence accepted under the certified assurance model says the external
  mutation applied but produced a domain failure, such as a finalized revert that consumed a nonce
  and fees;
- `VerifiedNotApplied`: evidence accepted under the certified assurance model says the exact
  invocation did not apply and can no longer be executed.

The classification is deliberately small. The referenced domain outcome artifact carries the full
typed result. `VerifiedAppliedFailure` is an external-effect classification, not necessarily a
failed MFM node: the owning state deterministically chooses its node settlement.

“Verified” means that MFM checked evidence integrity, provenance, binding, and deterministic
classification under the hash-bound verifier, finality policy, and trust assumptions. It does not
claim that ordinary RPC evidence independently proves unconditional external truth. The resolution
records the assurance contract digest, and public status exposes its safe domain assurance summary.
An unconditional proof claim is reserved for evidence whose cryptographic contract actually
supports it.

Reorganizations covered by the certified finality policy are handled before resolution and leave
the effect `Armed` with new checkpoints as needed. `Resolved` is immutable. A deeper
post-resolution reorganization is outside that run's recorded assurance and can be represented only
by a new fact, operation, or run linked through ordinary typed evidence.

Missing observations, timeout, provider unavailability, process death, signer unavailability, or
an absent lookup result leave the effect `Armed`. They never imply `VerifiedNotApplied`.

`Disputed` requires positive conflicting evidence, such as an external identity resolving to
content that cannot match the armed invocation. It stops automatic mutation, retains every fence,
and can return to `Resolved` only through evidence accepted by the certified verifier. It is not a
manual outcome. Like driver fencing, it cannot revoke a first-dispatch permit already issued to a
worker; the effect remains unresolved precisely because that invocation can still race with
reconciliation.

### Four effect event families

| Event | Meaning |
| --- | --- |
| `EffectArmed` | MFM durably fixed the logical effect, certified effect-contract reference, exact invocation commitment, witness, and optional serialization key before any external mutation call. |
| `EffectCheckpointObserved` | MFM retained materially new, typed, nonterminal evidence for the exact external operation identity. Repeated polling with no new evidence appends nothing. |
| `EffectDisputed` | Positive typed evidence conflicts with the armed identity or previously accepted evidence, so automatic driving must stop. |
| `EffectResolved` | The certified verifier accepted one terminal domain outcome and core classification. No second resolution is legal. |

These are event families, not an instruction to force every protocol into the same evidence schema.
Receipt, inclusion, execution, finality, nonce, UTXO, or bridge evidence remains domain-typed and is
referenced by a generic checkpoint or resolution event.

An effect event may record the emitting attempt for audit, but that attempt is never part of the
event's logical key or transition identity. A successor attempt appends to the same effect journal.

Every effect append requires a sealed `VerifiedEffectTransition` authority minted by the certified
runtime and domain verifier. It binds the run id, certified spec hash, `EffectId`,
`EffectContractRef`, expected stream head, transition payload hash, and every referenced evidence
hash. The store consumes that authority, applies structural legality, artifact-admission, driver
epoch, logical-key, active-key, and expected-head checks, and commits the transition atomically.
Schema-valid event bytes or artifacts alone can never enter the authoritative effect journal.

There is no generic event for prepared, invocation started, submission known, submission unknown,
receipt observed, or confirmed. Those names either describe transient execution or one protocol's
evidence shape. An unresolved `EffectArmed` already represents the full crash-safe uncertainty:
the invocation may or may not have reached the external system.

### `EffectArmed` is the sole mutation boundary

Before arming, runtime materializes certified input and context and invokes pure state logic to
author the typed intent and optional serialization key. Given that fixed material, the live adapter
may:

- perform bounded non-mutating preparation reads;
- select public invocation fields;
- request a transient signature;
- compute the external operation identity;
- compute a commitment to the exact invocation;
- validate the optional serialization key already derived by pure domain/state semantics.

Before arming, the adapter must not call an external mutation capability. This is enforced by
capability construction, not adapter convention: preparation receives read, signer, and support
capabilities only and returns a sealed prepared candidate to runtime. Runtime exposes a scoped
mutation dispatch permit only after the arm commit succeeds.

The certified node owns one hash-defining `EffectContractRef`. It resolves, through certified spec
authority, the adapter and replay-verifier identities, typed schemas, evidence provenance and
assurance rules, finality policy, recovery mode, serialization-key derivation, and any approved
reconstruction-provider identity. `EffectArmed` does not duplicate those policy fields as
independent event authority.

The `EffectArmed` commit atomically records:

- `EffectId`;
- node identity;
- the certified `EffectContractRef`;
- typed intent artifact reference and digest;
- external operation identity;
- an `InvocationCommitment`, defined as a domain-separated content digest over the exact transport
  bytes;
- a content-addressed, non-bearer `InvocationWitness` that lets the pure domain verifier bind the
  intent, signer or ordering identity, network, external operation identity, effect contract, and
  invocation commitment without reconstructing bearer bytes;
- optional single serialization key;
- an effect-specific non-bearer reconstruction or vault reference when the certified contract uses
  `RepeatExact`.

The same transaction acquires the optional entry in `active_effect_keys`.
The arm append requires the current driver fencing epoch as a commit precondition, but that
operational epoch is not part of effect identity or domain evidence.

An adapter that cannot provide a replay-verifiable, non-bearer `InvocationWitness` fails
certification. A witness that contains or reconstructs the exact submit-ready bearer is itself
bearer material and is rejected. Replay verifies the commitment's binding; it never needs the raw
invocation.

`EffectArmed` must never contain credentials, private keys, mnemonics, unlock material, bearer
tokens, signature scalars, or raw signed transaction bytes. Persisted structured material remains
canonical, content-addressed, and float-free.

Preparation returns a process-local `PreparedArm`. It contains the public arm evidence, external
identity, invocation commitment, witness, and the transient exact invocation. It is non-cloneable,
non-serializable, and zeroizes or destroys transient bearer material when dropped.

The arm operation has a closed outcome:

```text
NewlyAppended(FirstDispatchPermit)
AlreadyCommitted
Rejected
OutcomeUnknown
```

Only a newly appended arm consumes `PreparedArm` and mints a crate-private, non-cloneable
`FirstDispatchPermit`. An idempotent retry, reload, stale response, active-key conflict, rejected
append, commit-then-error response, or lost acknowledgement mints no permit. Orphaned pre-staged
content-addressed bytes are allowed, but no event may reference them until artifact admission,
effect arming, and key acquisition commit together.

An external mutation call is legal only through a `FirstDispatchPermit` or `ExactRepeatPermit`.
Mutation transport consumes one permit for at most one transport attempt and performs no hidden
retry. Observation contexts contain neither mutation nor signer authority, and permits cannot be
constructed from run history.

`EffectArmed` means that zero or more calls of the exact invocation may have occurred. It never
claims that a call started, reached a provider, or changed external state.

This distinction matters:

- a committed arm is durable authority that the exact invocation may already have been used;
- only `NewlyAppended` gives its current worker the one transient first-dispatch permit;
- reloading an arm never recreates that transient permit under `ObserveOnly`;
- `RepeatExact` may authorize another physical dispatch only under its stronger certified contract.

Immediately before transport entry, runtime rechecks the current driver epoch. If it is already
fenced, it destroys the unused permit. This is not absolute revocation: takeover can race after the
check, so the old call may still start or finish. Resolution and serialization-key release are
therefore legal only under evidence and assurance that remain safe against every delayed issued
permit.

### Certified recovery modes

Every write adapter must provide a certified observation path for the recorded external operation
identity. The spec binds one of two recovery modes.

For either mode, the reviewed adapter contract must bind the exact invocation to that identity and
exclude a delayed issued first call from becoming a different logical operation. Otherwise neither
driver fencing nor terminal resolution is safe and the adapter cannot be certified.

#### `ObserveOnly`

Recovery:

1. observes the recorded external operation identity;
2. verifies any returned evidence;
3. appends only materially new checkpoints, a dispute, or a terminal resolution;
4. gives a successor no mutation dispatch permit.

`ObserveOnly` does not prove that the original first-dispatch permit was unused or revoked. The
original worker can still race after takeover, so every later resolution must remain safe against
that outstanding permit.

If the first worker crashed after arming but before dispatch, and the domain cannot verify permanent
non-application, the effect can remain unresolved forever. This is a deliberate safety result, not
a retry bug.

#### `RepeatExact`

Recovery first observes at the current verified head and then produces one closed decision:

```text
Resolve | Dispute | Wait(checkpoint?) | DispatchExact
```

- `Resolve` and `Dispute` carry verified transition authority and append no mutation.
- `Wait` may carry a verified materially new checkpoint. It is otherwise required for lookup
  absence, unchanged nonterminal evidence, provider failure, reconstruction failure, or any
  unavailable prerequisite.
- `DispatchExact` is legal only after the approved provider reconstructs or retrieves the exact
  invocation and the verifier rederives its bytes digest, external identity, intent binding,
  signer or ordering identity, and certified policy binding.

Only `DispatchExact`, a current verified stream head, and a live driver epoch can mint a
non-cloneable `ExactRepeatPermit`. The transport consumes it once and then recovery returns to
observation.

Certification binds a reviewed `RepeatExact` contract and its assumptions; protocol conformance,
crash-race, and same-bytes tests justify enabling it. The contract must state that:

- exact invocation reconstruction remains possible after process loss;
- concurrent or delayed first and repeat calls all map to one external operation identity;
- overlapping physical calls cannot create a second mutation or separately meaningful cost-bearing
  external effect;
- no secret or bearer material enters ordinary persisted surfaces;
- provider identity and reconstruction behavior are hash-defining;
- a different nonce, fee, UTXO set, payload, signer, target, value, or encoded request cannot be
  substituted.

These guarantees remain conditional on the certified external protocol assumptions. If concurrent
and delayed calls cannot satisfy them, the adapter is `ObserveOnly`.

EVM mutation certification is blocked under this RFC until a concrete replay-verifiable,
non-bearer `InvocationWitness` is designed and passes review. `ObserveOnly` changes recovery
authority; it does not waive the witness requirement. The cutover retains reusable EVM transaction
construction, signing, transport, observation, and verifier primitives, but deletes the current
phase-ledger adapter and leaves `SubmitEvmTransactionState` unregistered.

After the witness contract qualifies, the first EVM recovery mode is `ObserveOnly`. EVM may become
`RepeatExact` only after a separate reviewed deterministic-reconstruction or secure-vault
capability satisfies the stronger contract above. The initial EVM contract also omits
`VerifiedNotApplied`: transaction lookup absence and nonce observation are insufficient. A future
EVM variant may add it only with assurance-backed permanent-invalidation evidence.

### Interruption remains a core lifecycle

Attempts are reduced to:

- `StateAttemptStarted`;
- `StateAttemptInterrupted`;
- `NodeSettled`.

`NodeSettled` is the one node-terminal event and records its typed success or terminal failure
outcome. A successful or failed settlement closes the active attempt without a separate
attempt-completed or attempt-failed phase. An interrupted attempt does not settle its node.

`StateAttemptInterrupted` means exactly:

> The recorded worker attempt no longer owns responsibility for driving this node.

It does not mean:

- the external invocation was not submitted;
- the effect failed;
- the effect may be replaced;
- compensation is owed;
- a serialization key may be released;
- the run is terminal.

An attempt may be interrupted before or after `EffectArmed`. If it is interrupted after arming, the
effect remains armed and its successor resumes the same `EffectId`.

There is at most one open attempt per run. A successor for an already armed effect must not rerun
intent authorship, preparation, signing, identity derivation, serialization-key derivation, or
arming. It loads the existing journal and enters only observation or certified exact repetition.
Any pre-arm transient bearer from an interrupted attempt is destroyed or zeroized and is never
transferred to a successor.

Pure and external-read attempts use the same interruption lifecycle without an effect journal. A
crash before their atomic settlement permits re-execution; only the committed settlement and its
retained evidence become authority. A repeated uncommitted read may observe a different external
frontier, which is acceptable because the earlier observation never committed. Replay uses only the
committed evidence and performs no new read.

### Driver lease and stale workers

A narrow per-run `DriverLease` remains mutable operational coordination. It has a fencing epoch and
is not replay, effect, or domain authority.

Controlled shutdown and takeover are distinct store operations.

Controlled shutdown atomically interrupts the one open attempt and relinquishes the lease. It does
not create a fictitious successor. If no attempt is open, it relinquishes only the operational
lease and appends no run event.

Idle acquisition covers a nonterminal run with no open attempt, including a crash after one node
settles but before the next starts. The current holder continuing normally, or a worker winning an
absent or expired lease, locks the lease and verified head, selects the one deterministic node to
drive, installs its current fencing epoch, and appends `StateAttemptStarted` in one
expected-head-guarded transaction. An unresolved effect makes its owning node the only eligible
choice; otherwise the ordinary graph frontier chooses the node. Idle acquisition appends no
`StateAttemptInterrupted` because no attempt was open. A terminal run or a head with no drivable
node rejects it.

Takeover locks the lease row and verified run head. It is legal only when that head contains exactly
one open attempt for an unsettled node and no competing settlement or takeover has won. In one
deterministically keyed, expected-head-guarded transaction, the store:

1. rejects the old fencing epoch for future appends;
2. appends `StateAttemptInterrupted` for the old attempt;
3. installs the successor fencing epoch;
4. appends `StateAttemptStarted` for the actual successor.

If the head no longer matches, takeover appends nothing and reloads.

Lease expiry alone never:

- resolves or resets an effect;
- releases `active_effect_keys`;
- proves non-submission;
- authorizes a different invocation.

A stale worker cannot append arm, checkpoint, dispute, resolution, or settlement with its old
epoch. It may still finish an external call already authorized by a committed arm. Consequently,
the certified adapter contract must ensure that any stale or repeated call can affect only the
recorded external operation identity.

Every driver-authored arm, checkpoint, dispute, resolution, and settlement append checks the
expected epoch while the store holds the lease-row lock in the same transaction. The lease fences
durable appends; it cannot revoke an external call after permit validation has raced with takeover.

### Crash recovery matrix

| Crash or ambiguous point | Required recovery |
| --- | --- |
| Before `StateAttemptStarted` commits | Start normally; no durable work exists. |
| After attempt start, before preparation completes | Interrupt the old attempt, start a successor, and recompute transient preparation. |
| Arm fails before commit | Destroy `PreparedArm`; no effect or key exists. Fresh preparation is required. |
| Active-key conflict during arm | Append nothing, destroy `PreparedArm`, and wait. Never reuse the stale candidate after the current holder releases. |
| Two workers prepare different candidates for one effect | Exactly one arm can be newly appended. Every loser destroys its candidate and receives no permit. |
| Arm commits but its acknowledgement is lost | Destroy the candidate and reload by deterministic logical key and fingerprint. The reloaded arm mints no first-dispatch permit. |
| After arm, before the first mutation call | Treat dispatch as unknown. `ObserveOnly` observes; `RepeatExact` observes and may then repeat the exact invocation. |
| Takeover between arm response and transport entry | The old worker rechecks its epoch and destroys the permit if already fenced. If takeover races after the check, the exact delayed call may still occur. |
| During the mutation call or after external acceptance but before response | Use the same armed-effect recovery. Never prepare a different invocation. |
| During an ambiguous checkpoint append | Reload the head, preserve the armed effect, and re-observe only if the checkpoint is absent. |
| A stale worker returns evidence after takeover | Its epoch cannot append. The current driver may reverify the evidence against the current head. |
| After terminal observation but before `EffectResolved` commits | Re-observe or reverify retained evidence, then attempt the atomic resolution commit. |
| Terminal artifacts stage but resolution fails | Orphaned bytes may remain, but no artifact authority, event, settlement, or key release commits. |
| During an ambiguous resolution append | Reload the terminal logical key. Never invoke after a committed resolution. |
| After resolution commits but before the caller receives a response | Return the folded committed result. |
| Positive identity or evidence conflict | Append `EffectDisputed`, retain all fences, and stop automatic driving. |
| Checkpoint, dispute, and resolution race | One expected-head transition wins. Every loser reloads and cannot downgrade or overwrite the winner. |
| Exact repeat races with resolution | Permit validation uses a current head and epoch; if resolution wins first, no permit is minted. A permit already past validation remains a possible delayed exact call covered by the assurance contract. |
| Resolution and key release win before a stale permit enters transport | The delayed exact call may still occur. The resolution assurance must make that call map only to the already resolved operation; `VerifiedNotApplied` additionally requires it to be non-executable. |
| Stale resolution retries after the key is reused | Owner-conditional release cannot delete the new holder's key row. |
| A delayed permit executes after `VerifiedNotApplied` | The non-application verifier must already have established non-executability despite that permit; otherwise resolution was illegal. |
| Lease expiry at any point | Expiry alone appends nothing. Controlled shutdown or a winning takeover may interrupt an open attempt; idle acquisition handles the no-open-attempt case. Every path preserves an unresolved effect and key exactly. |

### Store-enforced mutation fences

The store enforces structurally and atomically:

- one effect journal per certified `ApplySideEffect` node;
- at most one unresolved `Armed` or `Disputed` effect per run;
- every event transition requires matching `VerifiedEffectTransition` authority;
- no second arm or terminal resolution for one `EffectId`;
- no new effect may arm while another effect in the run is unresolved;
- no new effect may arm after a terminal node failure;
- an armed effect's node cannot settle until the effect resolves;
- a terminally closed run accepts no later run event;
- checkpoints, disputes, and resolutions must match the armed identity and certified schemas;
- a `VerifiedNotApplied` resolution requires purpose-specific authority minted only after the
  certified domain verifier accepts permanent non-executability under the assurance contract;
- stale driver epochs cannot append;
- the selected successful or failed effect-settlement batch commits all-or-nothing.

No trusted watermark may end inside either legal effect-settlement batch. The certified pure state
reducer selects the node settlement after consuming the verified effect resolution.

Successful node settlement:

```text
EffectResolved
+ resolution artifact admission
+ typed effect outcome
+ produced output cell
+ NodeSettled::Succeeded
+ owner-conditional active-key release
```

Failed node settlement:

```text
EffectResolved
+ resolution and typed outcome/error artifact admission
+ NodeSettled::Failed
+ owner-conditional active-key release
+ current-head-bound absorbing terminal closure
```

The failed shape creates no consumable output cell. A run may therefore become terminally failed
without a public output; failure rendering uses the reviewed redacted-error contract. Under the
serial driver contract, any `NodeSettled::Failed` establishes the failed run frontier, so its
closure metadata commits in the same batch. A pure or external-read node failure follows the same
atomic settlement-and-closure rule without effect resolution or key release.

A successful effect settlement does not close the run; later graph nodes and the render boundary
may remain. Successful run closure has its own atomic batch:

```text
public output artifact admission
+ certified public output
+ NodeSettled::Succeeded(PublicOutputRender)
+ current-head-bound absorbing terminal closure
```

Incremental folds apply the whole commit batch or none. Failure injection after every internal
database operation must leave no partial authoritative prefix. Artifact bytes may be staged before
the transaction and become orphaned after failure, but event references and artifact authority are
admitted only in the successful batch.

The forward mutation fence replaces saga engagement and obligation tracking. The initial runtime
admits only one open attempt per run, so no other node, including pure or external-read work, is
driven while an effect is `Armed` or `Disputed`. Once it resolves and its node settles, ordinary
graph scheduling may continue. Independently, no later mutation may cross its arm boundary while an
earlier mutation is unresolved or after any node has terminally failed.

A run cannot become terminal while an effect remains `Armed` or `Disputed`. Because node driving is
serial in this design, a separate node cannot newly fail while an effect attempt is unresolved.

`VerifiedNotApplied` is invocation-scoped. Its verifier authority binds the exact `EffectId`, source
arm event and fingerprint, invocation commitment, external identity, network or domain, signer or
ordering identity, `EffectContractRef`, evidence provenance, verifier, and finality policy. It must
accept both historical non-application and future non-executability despite every outstanding first
or repeat permit and reconstruction provider. Lease loss, interruption, timeout, mempool absence,
provider disagreement, and current lookup absence are categorically insufficient.

It does not say that another actor performed no equivalent business mutation. Any later fresh
invocation must re-read and verify its current domain preconditions before arming. An adapter whose
finality or invalidation model cannot support this classification omits it entirely.

### Narrow MFM-local serialization

Pure domain/state semantics may derive one optional serialization key from certified intent and
typed input. A live adapter may validate or encode it but cannot choose it. If live evidence is
required to derive the key, that evidence, its schema, and the deterministic derivation are retained
and hash-bound before arming.

Within one store scope, at most one unresolved armed effect can hold a given key. The store acquires
the key in `active_effect_keys` in the same transaction as `EffectArmed`. Each row binds the key,
owning `EffectId`, source arm commit identity, and arm fingerprint.

The key remains held through:

- process death;
- lease expiry;
- attempt interruption;
- provider or signer unavailability;
- unknown submission;
- dispute.

The key is released only by the atomic `EffectResolved` and `NodeSettled` commit, using a conditional
delete that matches the key, owner, source arm, and fingerprint. A stale or idempotent resolution
retry cannot release a later owner's row.

`active_effect_keys` is a store-authoritative constraint index inside the trusted MFM store
boundary, not a disposable projection or cache. Only the store transaction code, using a dedicated
database role, may mutate it. Normal mutation commits acquire locks in one order: driver lease, run
head, optional active key, and the store-wide commit-order allocator last. Those commits preserve
the journal/index relationship atomically and reject owner or fingerprint mismatches.

Startup enables arming only after the table is verified or rebuilt from committed effect journals
under exclusive writer exclusion. If startup or an explicit integrity audit detects a missing,
orphaned, wrong-owner, wrong-fingerprint, or watermark-inconsistent row, arming remains disabled
until offline repair completes. This RFC does not claim that every later out-of-band deletion is
synchronously detected by the next arm. Direct SQL mutation outside the store role and underlying
storage corruption are outside the certified execution guarantee. Memory and Postgres stores
implement the same API-level semantics.

The first design deliberately has:

- no FIFO waiter protocol;
- no TTL on an armed effect key;
- no claim takeover generation;
- no release event;
- no generic touched-set inference;
- no multi-key acquisition;
- no claim that external actors are fenced.

A contender can report that the key is active and retry later. Fairness and multi-resource
isolation require a concrete domain need and a separate design.

### Explicit compensation and corrective operations

Generic saga policy is removed from the certified spec.

An `ApplySideEffect` state produces a typed domain outcome from `EffectResolved`. Its certified pure
reducer decides whether that outcome settles the MFM node successfully or as a terminal failure.
`VerifiedAppliedFailure` is therefore successful external reconciliation, while the owning state
may still choose a failed node settlement that closes the run's forward mutation fence.

The minimal kernel does not add a conditional-choice or skipped-mutation primitive. A dependency-
ordered sequence of writes can remain in one run when every later write is valid after every
successful predecessor settlement. Conditional correction, replacement, and compensation use a
new, explicitly certified follow-up run.

An external caller may automate that launch from the retained typed source outcome, but the
follow-up still crosses a new planning, certification, identity, and admission boundary. Such
automation does not turn it into a hidden kernel obligation.

The follow-up has its own explicit invocation key. MFM makes no liveness or compensation guarantee
across the gap between source settlement and follow-up admission; if the caller crashes before
admission, no corrective run exists. A future durable initiation or outbox contract would require a
separate RFC.

The follow-up operation consumes ordinary immutable typed fact or artifact references from the
source run. Those inputs can include the source `EffectId`, resolution artifact, and verified source
stream coordinate. This is ordinary cross-run data lineage, not a kernel remediation link,
obligation, special run mode, or authority to rewrite the source history.

The follow-up run re-reads and verifies its current domain preconditions before arming its own
`ApplySideEffect` node. It may perform:

- correction after `VerifiedAppliedFailure`;
- a fresh invocation after `VerifiedNotApplied`;
- remediation after `VerifiedAppliedSuccess`.

Fee bumping, same-nonce replacement, or alternate live candidates while the original can still
execute are not ordinary correction nodes. They require a future adapter-specific multi-invocation
protocol.

A domain may publish `compensated` only with a typed equivalence claim that identifies:

- the source precondition or snapshot;
- the desired postcondition;
- external anchors or versions;
- touched resources;
- the assumed concurrent-writer model;
- the evidence establishing the predicate under those assumptions.

The certified verifier must validate that claim. Completing a corrective transaction without this
equivalence evidence may be reported only as `remediation_completed`. Core runtime status remains
neutral in both cases.

### Replace `ProjectionSnapshot` with `VerifiedRunView`

The public generic `ProjectionSnapshot` is deleted. `mfm-store` owns the sole private-field
`RunJournalFold` and all structural append legality. `mfm-runtime` owns an opaque
`VerifiedRunView` that composes one exact `RunJournalFold`, verified retained artifacts, and the
certified runtime spec without copying lifecycle maps. The existing verified-history boundary
evolves into these ownership layers; neither is introduced in parallel with the old projection.

Only the store can construct the structural fold, from:

- one `CommittedRunStream` at one exact stream head.

The fold contains only spec-independent per-run structural state:

- admission;
- open and interrupted attempts;
- node settlements;
- cells and referenced evidence;
- effect journals indexed by stable `EffectId`;
- public output state;
- absorbing terminal closure metadata, if committed;
- the exact stream head from which it was folded.

Store append validation uses this one fold. Runtime adds verified artifact, certified graph, and
domain-verifier authority to derive semantic run phase and mint `VerifiedEffectTransition`.
Replay and app consume the opaque runtime view; they cannot construct another lifecycle map.
Purpose-specific methods expose questions such as `unresolved_effect`, `settled_node`, or
`public_output`; callers do not receive a public bag of maps.

App status is rendered from one verified view. It cannot splice a run snapshot, saga snapshot,
attempt snapshot, and output snapshot loaded at different watermarks.

The semantic phase is small:

- `active`;
- `succeeded`;
- `failed`.

Any unresolved `Armed` or `Disputed` effect keeps the semantic phase `active`. Serial node driving
prevents a different node from newly failing while that effect is unresolved. Status separately
reports the effect phase and, after later terminal failure, whether the failed run contains prior
verified-applied effects. A persisted `Disputed` effect may be rendered as requiring intervention.

Provider, signer, vault, transport, or network unavailability is not persisted semantic phase. A
drive response may report advisory `waiting_external` or `blocked` for current operational
conditions, but replay does not reproduce it. Generic kernel modes no longer include `remediating`,
`compensated`, `manually_resolved`, or `failed_without_acdc_claim`.

### Keep only named non-run structures

A broad projection framework is not needed. Keep only structures with a distinct contract:

- `active_effect_keys`: atomic MFM-local mutation serialization authority, updated only with arm
  and resolution.
- `DriverLease`: mutable operational driver coordination with a fencing epoch.

`active_effect_keys` is a store safety constraint within the trusted store boundary; `DriverLease`
is operational coordination. Neither is run, replay, or external-truth authority. Any status
reading of either is advisory and append time always rechecks the relevant current row atomically.

The initial implementation deletes the physical fact projection and answers fact queries from
authoritative fact events at one repeatable-read `StoreCommitOrder` frontier, including source
artifact verification and deterministic ordering. A future acceleration index requires a separate
completeness contract and RFC.

Cross-run artifact dependencies remain explicit immutable settlement references. A synthetic
retention-manifest projection is not semantic authority. v1 retains indefinitely every artifact
referenced by admission, attempts, effect events, node settlements, facts, cross-run dependencies,
and public outputs. Artifact garbage collection is deferred until a future design can prove the
complete dependency closure.

### Ownership after the refactor

| Owner | Responsibility |
| --- | --- |
| State and operation | Deterministically construct typed intent, consume typed resolution, and author ordinary follow-up correction or compensation operations. No ambient IO. |
| Spec and certification | Bind `EffectContractRef`, adapter/verifier identity, schemas, assurance and finality policy, recovery mode, operation-identity derivation, pure serialization-key derivation, invocation-witness contract, and any exact-reconstruction provider. |
| Live adapter | Prepare transient invocation material, derive identity, commitment, and witness, validate the state-derived key, dispatch only by consuming a permit, observe, and stage typed evidence. |
| Replay verifier | Purely verify intent binding, invocation identity, checkpoints, finality, and resolution from retained evidence. |
| Transport | Perform reusable checked protocol IO with no hidden mutation retry. |
| Signer or secure reconstruction provider | Supply transient exact invocation material under an explicit certified capability. Never place secrets or bearer material in ordinary persisted surfaces. |
| Store | Own the sole `RunJournalFold`, append atomicity, logical-key idempotency, structural transitions, expected-head checks, artifact admission, driver fencing, the unresolved-run fence, absorbing terminal closure, and active-key constraint integrity. |
| Runtime | Own the opaque `VerifiedRunView` over the store fold, verified artifacts, and certified spec; start and interrupt attempts; enforce arm-before-call; follow recovery mode; resume the same effect; and settle only from verified resolution. |
| App, CLI, and REST | Start or resume driving, trigger certified observation, and render derived status. They do not decide external truth. |

### Replay contract

Replay:

- verifies the certified spec and certificate;
- folds the authoritative committed stream at one head;
- verifies retained artifact identity and bytes;
- recomputes state-authored intent and its digest;
- verifies the armed external identity and invocation commitment;
- verifies every checkpoint, dispute, finality condition, and resolution using the certified domain
  verifier;
- recomputes the typed node outcome and any success-required public output;
- reports unresolved armed effects as unresolved;
- performs no transport, signer, keystore, vault, current-config, or other live IO.

Replay cannot manufacture `VerifiedNotApplied`, replace an invocation, release a key, accept an
operator assertion as domain truth, or call a mutable verifier registry after certification.

There is no generic evidence-submission API. If a domain later needs proof ingress, its certified
contract must bind authenticity, provenance, effect prefix, schema, and verifier. Schema-valid bytes
alone cannot mint `VerifiedEffectTransition` or authorize `EffectResolved`; terminal resolution
still uses the epoch-checked atomic settlement path.

## Guarantee Changes

### Guarantees maintained

- Typed state programs remain the only semantic executable surface.
- Certified typed specs remain the only runtime contract.
- `ApplySideEffect` remains the explicit effect class for external mutation.
- States and operations perform no ambient IO.
- Adapters alone bind typed intent to mutation capabilities.
- The `run:*` stream remains append-only and authoritative.
- Each store append remains all-or-nothing.
- Manifests, facts, artifacts, outputs, specs, intents, and evidence remain content-addressed where
  required by their contract.
- Hashed structured values remain canonical and float-free.
- Secrets and bearer mutation material remain absent from events, artifacts, outputs, errors, and
  logs.
- Replay remains evidence-only and constructs no live capability.
- Worker processes remain fungible; process identity is never semantic truth.
- Interruption remains durable and resumable.
- Finality and verification policy remain hash-defining certified material.

### Guarantees strengthened or added

- Capability construction exposes no external mutation call before a newly appended
  `EffectArmed` mints a one-shot permit.
- Every mutation node has one stable effect identity independent of attempts.
- Arming fixes one exact invocation commitment and external operation identity.
- After `EffectArmed` commits, recovery cannot prepare a semantically different invocation.
- `RepeatExact` exists only under a hash-bound, reviewed adapter contract and its explicit external
  assumptions.
- Missing or unavailable observation never becomes a false `VerifiedNotApplied`.
- `VerifiedNotApplied` requires assurance-backed evidence that every outstanding stale invocation
  cannot later execute.
- One effect receives at most one terminal resolution.
- Attempt interruption never resets or replaces effect truth.
- At most one unresolved mutation exists per run.
- No later mutation arms after terminal node failure or while an earlier mutation is unresolved.
- Within one store scope, an optional matching serialization key has at most one unresolved effect
  owner and survives worker loss.
- Resolution, typed outcome, the selected success-or-failure node settlement, artifact admission,
  and key release are atomic; only successful settlement produces a consumable output cell.
- The first derived terminal commit append-closes the run; no later event can change its outcome.
- Per-run status, scheduling, resume, and replay use one verified stream watermark.
- Positive evidence conflict has an explicit fail-closed `Disputed` state.
- Correction and compensation are visible in separate certified typed operations rather than
  inferred by kernel policy.

### Guarantees and claims removed

- Generic certified saga semantics for every side-effecting run.
- Generic AC/DC-equivalence language.
- Automatic saga engagement and reverse-order remediation.
- Kernel-derived compensation obligations.
- Automatic or durable initiation of a corrective or compensating run after source settlement.
- A generic `Compensated` outcome.
- A generic `ManuallyResolved` outcome.
- `FailedWithoutAcdcClaim` as a special terminal mode.
- The claim that signed operator authorization establishes external domain truth.
- Generic receipt-versus-confirmation lifecycle phases.
- Generic resource policies such as exact touched sets or manual-only isolation.
- FIFO resource-waiter fairness.
- Claim takeover as mutation authority.
- In-place retry, replan, fee bump, or replacement of an armed invocation.
- Any implication that interruption proves non-submission.
- Any implication that a projection row independently authorizes resume, replay, settlement, or
  status.

MFM explicitly does not claim physical exactly-once invocation, arbitrary external exactly-once
behavior, rollback, cross-chain atomicity, perpetual finality, external-actor serialization,
generic compensation equivalence, guaranteed follow-up initiation, or tolerance of writes that
bypass the trusted store boundary.

## Example Lifecycles

### Ordinary successful write

```text
StateAttemptStarted
EffectArmed
external dispatch
EffectCheckpointObserved        # optional
EffectResolved(VerifiedAppliedSuccess)
NodeSettled + typed output      # same commit as resolution
```

### Crash immediately after arming

```text
StateAttemptStarted
EffectArmed
process dies
StateAttemptInterrupted
StateAttemptStarted             # successor
observe the same external identity
```

From there:

- an `ObserveOnly` successor waits for verifier-accepted evidence and receives no dispatch permit;
- `RepeatExact` observes first and may reconstruct and dispatch only the exact invocation;
- missing evidence leaves the effect armed;
- conflicting evidence makes it disputed.

### Expected compensation

```text
source run: effect A resolves VerifiedAppliedSuccess
source run settles and retains A's typed resolution evidence
follow-up operation consumes ordinary immutable source evidence
follow-up run rechecks current domain preconditions
follow-up effect B arms and resolves
domain verifier accepts an equivalence claim, or reports remediation_completed
```

The kernel reports two independent resolved effects in two certified runs. Only the typed
equivalence contract described above authorizes the domain word `compensated`.

### Unexpected failure after an applied mutation

```text
effect A resolves VerifiedAppliedSuccess
later node settles as terminal failure
run phase becomes failed
status reports prior applied effect A
no later effect in this run may arm
```

An operator can inspect evidence, resume observation where legal, or start a separate certified
remediation run. MFM does not silently claim rollback or compensation.

## Complete Cutover and Deletion Scope

This is a replacement, not a migration with parallel paths.

### Retain and simplify

- `ApplySideEffect` and one explicit external mutation capability per such state;
- state-authored typed intent and typed outcome;
- certified adapter, verifier, observation, finality, and recovery contracts;
- reusable domain transaction construction, signing, transport, observation, and replay
  verification primitives; EVM mutation registration remains disabled until its witness contract
  qualifies;
- attempts and durable interruption in their reduced form;
- committed streams, atomic appends, artifacts, canonical hashing, and content addressing;
- frontier-bounded fact queries over authoritative fact events;
- successful public output derived from typed terminal cells.

### Program, spec, and certification

Delete:

- generic saga policy;
- forward and remediation roles, pair ids, and handles;
- compensation collections and obligation policy;
- generic manual-resolution policy and schema authority;
- the `mfm-manual-auth` crate, verifier registry, schema roles, and proof types;
- generic receipt-versus-confirmation policy enums;
- synthetic `SideEffectVerify` and `ResolveSagaTerminal` nodes;
- the current phase-ledger-coupled `SubmitEvmTransactionState` certification and runtime
  registration; its reusable EVM domain and transport primitives remain available for
  requalification.

Add:

- one certified `EffectContractRef` per `ApplySideEffect` node;
- stable operation-identity derivation;
- invocation-commitment contract;
- replay-verifiable non-bearer invocation-witness contract;
- typed observation and resolution schemas;
- evidence provenance and assurance summary;
- adapter-specific finality and recovery mode;
- optional pure one-key serialization derivation;
- optional exact-reconstruction provider identity.

### Events and store

Replace the current side-effect event family with:

- `EffectArmed`;
- `EffectCheckpointObserved`;
- `EffectDisputed`;
- `EffectResolved`.

Reduce node execution lifecycle to:

- `StateAttemptStarted`;
- `StateAttemptInterrupted`;
- `NodeSettled`.

Delete:

- side-effect pair ids, roles, purposes, and epochs;
- distinct intent, claim, prepared, started, submission-known, submission-unknown, receipt, and
  confirmation kernel events;
- saga engagement, obligation, remediation, manual-resolution, and saga terminal events;
- `SagaTerminalProof`;
- `RunCompleted`, `RunCompletionOutcome`, `StateAttemptCompleted`, and `StateAttemptFailed`;
- `Retention*` events and obsolete side-effect/manual/retention artifact roles;
- the retryable open-attempt disposition algebra;
- generic resource-lane claim, takeover, release, touched-set, FIFO waiter, and generation
  protocols;
- public `ProjectionSnapshot` and projection-specific mutation DTOs;
- projection-only retention manifests.

Add:

- structural effect-journal validation;
- unresolved-effect and forward-arm fences;
- expected driver fencing epoch preconditions;
- `active_effect_keys`;
- `VerifiedEffectTransition` and resolution sub-authority;
- owner-conditional, fingerprint-bound key release;
- current-head-bound absorbing terminal closure metadata;
- direct immutable cross-run artifact dependency references where required.

### Runtime and replay

Delete:

- saga scheduler decisions and modes;
- remediation-order derivation;
- generic manual-block recovery;
- phase-specific side-effect recovery branches;
- replay brokers that mimic live mutation phases;
- `CompleteRun` and `ProjectRetentionManifest` framework nodes and their commit purposes;
- synthetic completion, verification, and retention scheduling whose result is derivable from
  certified graph settlement.

Retain `PublicOutputRender` as the certified successful-domain render boundary; it is not
run-terminal authority. Run terminality and its `StoreCommitOrder` coordinate are derived from the
first complete fold with no unresolved effect and a graph-terminal settlement. Successful
terminality additionally requires the certified public output. Failed terminality requires the
complete failed settlement batch and no public output cell. No semantic wall-clock timestamp is
invented.

The terminal frontier is absorbing. `PublicOutputRender` is schedulable only when every required
non-render node has settled successfully and no effect is unresolved; its output and successful
settlement commit together. A failed graph never schedules it. At the first successful or failed
terminal frontier, a current-head-bound verified terminal authority places an absorbing closure
flag in the same stream commit envelope. This is structural commit metadata, not a standalone
`RunCompleted` event or independent semantic outcome. The store folds the flag and rejects every
later run append. Replay rederives terminality from the certified graph, evidence, settlements, and
success output and rejects a missing, early, or mismatched closure flag.

Add:

- `PreparedArm`, `FirstDispatchPermit`, `ExactRepeatPermit`, and the arm-before-call runner protocol;
- adapter-private signer and mutation capabilities, observation-only recovery contexts, and
  permit-consuming mutation transports;
- `ObserveOnly` and `RepeatExact` recovery drivers;
- the crash matrix in this RFC;
- the store-owned `RunJournalFold` and runtime-owned opaque `VerifiedRunView`;
- one pure domain-verifier path shared by live settlement and replay;
- derived run terminality from certified graph, node settlements, unresolved effects, and, for
  success, certified public output.

### App, CLI, and REST

Remove generic saga, remediation, resource-lane, manual-resolution, and attempt-disposition DTOs,
commands, schema roles, and routes. Remove generic `SigningCapability` exposure to states, combined
EVM submit/observe capability surfaces, saga fields and modes from run/replay responses, and the
old execution-claim/run-admission paths replaced by `DriverLease`.

Expose:

- run phase;
- current open attempt, if useful operationally;
- unresolved effect identity and safe recovery status;
- resolved effect summaries, assurance summaries, and whether a failed run contains prior
  verified-applied mutations;
- redacted, verifier-safe intervention requirements;
- one explicit resume or observe action.

No process-facing API may expose bearer invocation bytes, secrets, provider credentials, or a
manual “mark successful/not applied” operation. There is no generic evidence-submission endpoint.

### Postgres

Reset the pre-production schema baseline.

Delete:

- generic projection tables and codecs;
- saga, remediation, obligation, and manual-resolution structures;
- resource claims, claim transitions, FIFO waiter tables, and takeover generations;
- obsolete side-effect phase columns and indexes;
- the physical fact projection;
- old artifact roles, schema registries, projection codecs, and event decoders.

Retain authoritative commits, run events, retained artifacts and evidence, and store-wide commit
ordering. Fact queries scan authoritative fact events at one repeatable frontier. Add only the
driver lease and owner-bound active-effect-key structures required by this RFC.

The schema provisions a dedicated MFM store-writer role as the only non-administrative role with
`INSERT`, `UPDATE`, or `DELETE` privileges on `active_effect_keys`. Application, query, migration
consumer, and other runtime roles receive no direct DML grant. Administrative bypass and storage
corruption remain outside the certified boundary described above.

Old histories are rejected explicitly. There are no legacy event decoders, compatibility aliases,
dual writers, shadow recovery paths, or schema fallbacks.

### Documentation

On acceptance and implementation:

- rewrite the mutation, runtime, store, replay, and authority sections of
  [docs/design.md](docs/design.md);
- update placement and ownership in [docs/architecture.md](docs/architecture.md);
- rewrite [docs/evm-transactions.md](docs/evm-transactions.md) to record that mutation registration
  is blocked until the new witness contract qualifies, then document the new journal;
- update the persisted/public surface inventory and affected binary READMEs;
- delete [docs/saga.md](docs/saga.md) after its current contract is fully removed;
- keep this RFC as the decision record or mark it implemented.

## Required Tests and Acceptance Criteria

### Crash and interruption

- Inject a crash at every row of the crash matrix for memory and Postgres stores.
- Prove controlled shutdown appends exactly one interruption and no successor, while a winning
  expired-lease takeover appends exactly one interruption and one real successor.
- Prove idle acquisition after a node settlement or controlled shutdown starts exactly one
  deterministic next attempt and fabricates no interruption; when an effect is unresolved, it can
  start only that effect's owning node.
- Prove pre-arm interruption discards preparation and lets a successor create the effect's sole
  invocation commitment.
- Prove post-arm interruption never changes `EffectId`, reruns preparation, or creates or replaces
  the invocation commitment.
- Prove an interrupted armed effect remains armed.
- Test stale worker races at prepare, arm, dispatch, observe, resolve, and settle boundaries.
- Prove an ambiguous arm or resolution append recovers by logical key and payload fingerprint.
- Recreate services, adapter instances, signer/reconstruction handles, database connections, and
  all transient caches after every simulated process crash.

### Mutation safety

- Compile-fail or type-test every external-call path without `FirstDispatchPermit` or
  `ExactRepeatPermit`.
- Prove only `NewlyAppended` can consume `PreparedArm` into `FirstDispatchPermit`.
- Reject first-dispatch invocation, identity, witness, or commitment mismatch.
- Use transport spies to prove one protocol exchange per consumed dispatch permit and no hidden
  retry.
- Prove an `ObserveOnly` successor performs zero dispatches while allowing the original issued
  permit to race.
- Reject `RepeatExact` without its approved provider.
- Reject exact-repeat reconstruction on any byte, digest, identity, signer, or policy mismatch.
- Prove missing lookup never produces `VerifiedNotApplied`.
- For every adapter that supports `VerifiedNotApplied`, use a domain simulator and conformance suite
  to exercise its historical non-application and future non-executability assumptions, including
  delayed permits.
- For every adapter that omits `VerifiedNotApplied`, reject every attempted resolution of that
  class.
- Exercise `RepeatExact` overlap between the original permit, stale repeats, current repeats, and
  resolution in a domain conformance simulator.
- Exercise `arm appended -> takeover -> resolution/key release -> stale permit transport entry`;
  reject the resolution unless its assurance remains valid against the delayed exact call.
- Cover verified applied success, applied failure, conflict, finality, and pre-resolution
  reorganization evidence for every adapter. Test post-resolution reorganization only as a new
  linked fact, operation, or run.
- Prove signed bytes, bearer material, credentials, and secrets never enter persisted or public
  surfaces.

### Store fences and atomicity

- Reject every effect event without a matching, current-head `VerifiedEffectTransition`.
- Reject schema-valid evidence bytes as transition authority.
- Reject two unresolved effects in one run.
- Reject a new arm after terminal node failure.
- Reject `NodeSettled` for an unresolved armed or disputed effect; allow exactly one
  verifier-authorized `EffectResolved` transition from `Disputed`.
- Test active-key contention across runs.
- Prove lease expiry and interruption do not release an active key.
- Prove dispute retains the key.
- Test owner/fingerprint-bound active-key ABA safety: after E1 releases K and E2 acquires K, a stale
  E1 resolution retry cannot release E2's row.
- Prove the normal store API cannot create journal/key-index divergence and rejects current-row
  owner or fingerprint mismatches.
- Seed missing, orphaned, wrong-owner, wrong-fingerprint, and watermark-inconsistent key rows before
  startup; prove integrity verification keeps arming disabled until an offline rebuild under writer
  exclusion succeeds.
- Verify Postgres grants and exercise denied DML attempts to prove non-store application roles
  cannot insert, update, or delete `active_effect_keys`; keep administrative bypass explicitly
  outside the guarantee.
- Treat direct out-of-band table changes as explicit corruption fixtures, not as a continuously
  enforced runtime guarantee.
- Inject failure after every database operation in both legal effect-settlement shapes. Prove
  resolution artifact admission, event append, typed outcome, `NodeSettled`, and key release are
  all-or-nothing; include output-cell production for the successful effect shape and prove the
  failed shape produces no output cell and cannot commit without terminal closure.
- Inject failure through public-output artifact admission, render settlement, and terminal closure;
  prove successful terminality commits all three or none. Apply the same closure atomicity test to
  pure and external-read terminal failures.
- Race checkpoint versus resolution, dispute versus resolution, checkpoint versus dispute, two
  resolutions, and resolution/key release versus a new arm.
- Test memory/Postgres transition parity.

### Replay and views

- Replay every effect outcome with zero live IO.
- Reject tampered intent, invocation identity, checkpoints, finality evidence, outcome, and
  resolution.
- Prove full-history and incremental `RunJournalFold` equivalence and identical
  `VerifiedRunView` queries over either result.
- Prove per-run status, scheduling, resume, and replay consume one stream watermark.
- Prove successful run terminality requires its certified public output, while failed terminality
  requires no public output and fabricates no consumable output cell.
- Reject `PublicOutputRender` before the graph-success frontier, reject it for a failed graph, and
  reject every event append after the absorbing terminal closure flag commits.
- Reject a missing, early, or mismatched terminal closure flag during replay.
- Prove projection or cache corruption cannot independently authorize a transition.
- Test fact queries against authoritative events at one repeatable `StoreCommitOrder` frontier.

### Explicit correction

- Execute a source run with a typed failure and a separately certified follow-up correction run.
- Execute a follow-up remediation run with no generic saga event or mode.
- Reject a follow-up run whose ordinary source fact/artifact evidence is missing, unverified, or
  does not identify a resolved source effect.
- Prove source settlement creates no hidden follow-up obligation: a correction starts only through
  ordinary admission with its own invocation key, and a caller failure before admission leaves no
  MFM liveness or compensation claim.
- Require a verified typed equivalence claim before rendering `compensated`; otherwise render only
  `remediation_completed`.
- Prove an unexpected terminal failure after an applied mutation reports that history and arms no
  later mutation.
- Compile-fail an `ApplySideEffect` state without exactly one certified mutation authority,
  `EffectContractRef`, operation-identity and invocation-witness contracts, verifier, assurance and
  finality policy, and recovery mode.

### EVM adapter qualification gate

The core refactor may be accepted while EVM mutation remains unregistered. Re-enabling
`SubmitEvmTransactionState` requires all of the following:

- Define a concrete non-bearer `InvocationWitness` and prove its binding to the exact transaction
  commitment, transaction hash, intent, signer, chain, and `EffectContractRef`.
- Reject witness tampering, cross-run replay, cross-chain replay, signer substitution, intent
  substitution, policy substitution, and any witness from which bearer transaction bytes can be
  reconstructed.
- Cover interruption before and after arm.
- Cover loss of submit response and receipt.
- Cover already-known transaction behavior.
- Cover successful and reverted receipts.
- Cover configured finality and pre-resolution reorganization evidence.
- Cover initial `ObserveOnly` recovery without treating missing lookup as non-application.
- Cover `RepeatExact` only if an approved deterministic reconstruction or secure provider later
  qualifies.
- Prove raw signed transaction bytes remain transient.

The implementation is accepted only when the old saga, phase-ledger, resource-lane, universal
projection, and EVM phase-ledger registration paths are deleted. Passing tests through a retained
compatibility path is not acceptance. The core may ship with EVM mutation unregistered; EVM writes
return only after the qualification gate above passes.

## Ordered Logical Commits

1. **`query facts from authoritative event history`**

   Remove the physical fact projection and its codecs/tables. Preserve current fact-query behavior
   by scanning authoritative fact events at one repeatable `StoreCommitOrder` frontier and
   revalidating source artifacts. Update the current design, schema baseline, and focused tests
   without changing mutation or saga semantics.

2. **`replace saga recovery with stable effect journals`**

   Perform one inseparable vertical cutover across program/spec/certification, capability types,
   events, store, attempts, driver lease, active keys, runtime, replay, the EVM qualification gate,
   `VerifiedRunView`, artifact retention, public outputs, app/CLI/REST, Postgres, schemas, tests,
   and documentation.
   Introduce the effect journal, transition authorities, dispatch permits, assurance contract, and
   crash protocol while deleting saga, manual resolution, the phase ledger, resource lanes,
   execution claims, generic projections, synthetic completion/retention, and every old public or
   persisted surface.

Each commit updates its code, tests, authoritative design documentation, architecture placement,
persisted/public surface inventory, and API documentation. Inseparable schema and producer/consumer
cutovers stay in the same commit. Internal preparatory commits are acceptable only when they
preserve the one current contract unchanged; no intermediate compatibility or parallel mutation
path is allowed.

## Alternatives Rejected

### Make the certified core read-only

Rejected because writes are part of MFM's platform purpose. It would move recoverability into every
future product or adapter and leave the core unable to make a consistent mutation guarantee.

### Keep the current ledger and only fix known bugs

Rejected because the carrying cost comes from the number of concepts and duplicated lifecycle
models, not one isolated defect. Local fixes preserve the phase cross-product and future change
sites.

### Promise generic exactly-once mutation

Rejected because a process cannot distinguish “crashed before dispatch” from “dispatched and lost
the response” without domain support. Physical duplicate calls may also occur below MFM. The honest
portable guarantee is stable logical identity plus observe-only or certified exact-repeat recovery.

### Persist raw signed invocations in ordinary artifacts

Rejected because signed transactions are bearer mutation material. Making them ordinary run
artifacts would violate the existing secret and capability boundary and expand the blast radius of
store access. Any secure reconstruction or vault design must remain an explicit reviewed
capability.

### Keep generic saga but rename its claims

Rejected because obligation derivation, reverse remediation order, manual terminalization, and
generic compensation remain domain semantics even under weaker names. Explicit certified follow-up
operations are more auditable and make no automatic equivalence claim.

### Treat interruption as proof that a write did not happen

Rejected because interruption describes worker ownership only. A crashed or fenced worker can have
already changed the external system or can finish an already authorized call.

### Release mutation keys when a lease expires

Rejected because a driver lease protects local appends, while an armed invocation can outlive the
worker and still execute externally. Only verifier-accepted effect resolution can release the
mutation key.

## Decision Summary

MFM will keep a durable write kernel, but that kernel will guarantee only what it can substantiate
under its recorded assurance contracts:

- durable authority before mutation;
- one stable logical effect;
- no different invocation after arming;
- observation or reviewed, contract-bound exact repetition after interruption;
- verifier-backed terminal resolution;
- strict forward and serialization fences;
- atomic settlement;
- evidence-only replay.

Business correction, remediation, and claims about restored state must be expressed by separately
certified typed domain operations and their evidence. Live replacement remains outside this kernel
until an adapter-specific multi-invocation protocol is designed.
