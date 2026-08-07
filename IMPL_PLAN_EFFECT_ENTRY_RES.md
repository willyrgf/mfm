# Implementation plan: effect entry resolution

Target design: [`docs/effect-entry-resolution.md`](docs/effect-entry-resolution.md) — the
`EffectEntryMode` absorption axis. The direction is not yet accepted; see *Gate before commit 5*.

Gaps closed: *No generic recovery from a possible external entry*, *A crashed Read attempt strands
its run*, and *No way to find a parked run* in [`docs/known-gaps.md`](docs/known-gaps.md).

[`AGENTS.md`](AGENTS.md), [`docs/code-quality.md`](docs/code-quality.md), and
[`docs/build-and-verification.md`](docs/build-and-verification.md) are in force.

This plan replaces two earlier ones: a separate read-only probe component, and a re-invocation path
gated on `Refreshable`. Both are recorded under *Rejected alternatives* in the design doc. The residue
matters here because both commit lists were traps — the probe plan looked like five additive commits
and was seven invariant relaxations across five fold functions; the `Refreshable` plan's two
"relaxations" included the strict-descendant rule in the physical-binding verifier, the highest-risk
edit available. **This plan relaxes exactly one rule, in the Read path only.** Everything on the
Effect path is additive. If an edit here starts to look like a relaxation, that is the signal to stop,
not to widen the diff.

---

## Gate before commit 5

Commits 1–4 are useful, additive, and reversible under any of the designs considered. Commit 5 is the
one that spends the property, and it must not be written before the trade is accepted in writing:

> Today one entry per occurrence is **structural** — the legal ordinal is derived from the leaf and
> any authorization while an unobserved one exists is rejected, so an Effect occurrence reaches the
> external system at most once, ever, in any deployment, regardless of adapter correctness. After
> commit 5 the bound is `MAX_ENTRIES`, and containment of the extra `MAX − 1` entries lives in a
> declaration nothing can check.

If that trade is judged unacceptable, stop after commit 4 and implement *Authored recovery routes*
from the design doc instead: commits 1–4 are all still wanted, and none of them presumes re-entry.
Commit 4's certified `EntryOnce`/`EntryAbsorbing` field is the only sunk cost, and even that survives
as a reportable per-capability property.

## Decided while writing this plan

Two corrections landed in the design doc rather than staying open here. Both change what gets written,
so they are recorded where the mechanism is specified, not only in a commit body.

- **Two leaves, not one.** `Reassertable` cannot cover a *crashed* attempt, because the closing
  observation must commit first and a leaf that already says "re-assert" gives Runtime no instruction
  to close. A crashed absorbing attempt folds to `EntryClosable { access_attempt_id }`, which
  authorizes nothing; only its committed closure folds to `Reassertable`. The design doc's *Behaviour*
  section previously read as though one leaf sufficed.
- **The Read path relaxes the outstanding-attempt rule, and the design doc now says so.** That rule
  (`fold.rs:4018-4029`) rejects a new authorization if *any* attempt of the occurrence has an
  authorization and no observation, unconditionally across access kinds — so admitting a Read successor
  with its predecessor standing is that rule relaxed. Sound, because a Read consumes nothing, but it is
  a relaxation and the doc had it as a property that already held. It must be an
  access-kind-conditional check with its own rejection message and its own regression on both halves.
  The Effect path keeps the rule verbatim, which is the entire purpose of the closing observation.

## Material uncertainties

- **Runtime mints a fault code it did not observe.** The closing observation is
  `ObservationOutcome::EntryUnknown { fault_code }`, and the fold validates nothing about that code
  beyond `access_kind == Effect` (`fold.rs:4440-4444`), so the record is legal today with no contract
  change. But the code is the *only* thing distinguishing a Runtime-synthesized closure from an
  adapter-reported ambiguity anywhere downstream — the record shape is identical
  (`journal/src/structured.rs:1101-1108` carries no origin). Reserve one kernel-owned `StableId` for
  it, never reachable from an adapter, and assert in the audit projection test that the two are
  distinguishable. Do not let a domain reuse it.
- **Whether closing is gated on remaining budget.** This plan gates it: Runtime closes only when the
  capability declares `EntryAbsorbing` *and* the ordinal is below `MAX_ENTRIES`, so the closure always
  has a consumer, per the design doc's scoping argument. The cost is that a budget-exhausted park
  keeps a dangling authorization in history, so the doc's "every attempt in a resolved history carries
  a terminal observation" holds only for *resolved* histories. The alternative — always close a
  crashed absorbing attempt, then park on the `EntryUnknown` leaf — makes history tidier and the
  trigger condition wider. Decide in commit 5; do not leave both paths in the code.
- **`Returned` must be post-state-functional and nothing checks it.** `RowsAffected(1)` violates it;
  `InsertOutcome { key, row }` satisfies it. It is load-bearing for every absorbed repeat and is
  checkable by review only. Not resolvable by this work. State it in the design doc's certification
  obligations and in each EVM capability's rustdoc.
- **`MAX_ENTRIES` values are a policy choice with no evidence.** Start at 3 and revisit with the
  qualification matrix.
- **Windowed absorption is undetectable here.** The fold has no clock and cannot have one without
  breaking refold equivalence, so no TTL field is added. Commit 2 is the mitigation and is therefore
  not optional: discovery latency is the whole exposure.

---

## What is removed from EVM, and what is not

Investigated construct by construct. Unchanged from the previous plan — **no expansion construct is
deleted under any of these designs** — and the reason is the gap itself: EVM has no in-run handling of
`EntryUnknown`. Its adapters classify the ambiguity (`live/evm/src/structured.rs:451,460`;
`evm-postgres/src/authority.rs:570,578,641,661,683,704,717,720`) and the run parks.

| EVM construct | Question it answers | Subject | Fate |
| --- | --- | --- | --- |
| adapter `EntryUnknown` classification | did *my* effect enter? | this occurrence | kept; becomes the re-assertion trigger |
| *nothing* | how do I resolve that? | this occurrence | **the gap**; new behaviour, not a replacement |
| `CandidateSlotRoute::ObserveRetained`, `PrepareRetainedCandidateObservationState`, retained branch of `select_candidate_attempt_route` (`submission_process.rs:758-780`) | did *any* candidate in the family win the nonce race? | the candidate family | **survives** |
| `PendingEvmSubmissionFailure::Reconcile`, `reconcile_absent`/`reconcile_reserved` (`submission_process.rs:580-602`), both `CustomFailureHandler` reconcile arms, `ReadReservationStatusAfterFailureState`, `ReadCandidateStatusAfterFailureState`, `ReadExhaustionStatusState` | has another run already reached a terminal answer for this shared reservation? | the shared reservation | **survives** |
| `CandidateActivationDecision::Reconcile` + `MarkActivationReconcileState` + `candidate-activation-reconcile` arm + `ReadCandidateWalletNonceStatusState` | did another writer get to this nonce first? | the shared reservation | **survives**, narrower reachability |

`ObserveRetained` reads as adoption of an abandoned run's candidate and does serve that, but
`SubmissionWork` gives away its real purpose: `next_candidate_ordinal` is "next family ordinal to
visit in certified order (recovery starts at 0)" and `observed_prefix_len` bounds "this recovery
walk" (`submission.rs:1273-1276`). The walk revisits every activated ordinal, including ones the
same run activated, because an older activated replacement may still win. Deleting it breaks
candidate-family semantics.

`CandidateActivationDecision::Reconcile` fires on `CandidateProgressionConflict` and on
`AlreadyRetained` (`submission_process.rs:1054-1091`). The `AlreadyRetained` branch is today reached
partly because a parked occurrence's own earlier write is rediscovered by a fresh run. After commit 6
the parked run resolves itself, so that reachability disappears; the genuine "another writer got here
first" case remains.

**`AlreadyRetained` is also the working proof that absorption is the right axis.** It is already an
adapter re-invocation that reaches the same key, finds its own write present, and reports it — the
exact behaviour commit 6 generalizes. It is not invented here, and it did not need a lineage head, an
exclusive slot, or a negative read to be correct. That is the evidence behind the design doc's claim
that `Refreshable` was over-priced for this job.

**Therefore no EVM expansion construct is deleted.** Adding a declaration to four capabilities is not
a parallel implementation of the surviving constructs, because none of them resolves a per-occurrence
authorization.

### Adjacent duplication found, deliberately out of scope

`ReadWalletNonceStatusState`, `ReadPostReserveWalletNonceStatusState`,
`ReadReservationStatusAfterFailureState`, `ReadCandidateStatusAfterFailureState`,
`ReadCandidateWalletNonceStatusState`, `ReadObservedCandidateStatusState`, and
`ReadExhaustionStatusState` (`submission.rs:1598-1661`) are seven states over one capability, one
request type, and one returned type, differing only in input shape and projection.
`FailureReconciliationRequest`, `PrepareExhaustionReconciliationState`, and
`MarkActivationReconcileState` exist only to reshape inputs for them. Real simplification target,
unrelated to this cutover. Record it; do not fold it in.

---

## Reference types

The complete new surface. Six items, none of which touches `EffectRefreshMode`.

### `mfm-capabilities` — the entry key

```rust
/// A request that carries the exact value the external system keys this effect on.
///
/// Two properties of this signature are load-bearing and must not be relaxed:
///
/// - `&self` is the only input, so no ambient IO and no process state can enter
///   the derivation. The key is a pure projection of committed request bytes and
///   outlives the process that authored them.
/// - It is total. A request cannot carry `key: Option<K>` and implement this
///   honestly, so declaring absorption forces the field to be mandatory in the
///   request shape. Making it fallible would let a capability declare absorption
///   it cannot always name, and strand at runtime — which is the autoincrement
///   case, and it must fail to compile rather than park.
pub trait EntryKeyed: MfmValue {
    /// Exact value the external system keys this effect on.
    type EntryKey: MfmValue + Eq;

    fn entry_key(&self) -> Self::EntryKey;
}
```

Per-state variation — different tables, columns, predicates — reaches the adapter as data in the
committed request, never as a second callback. The state already authored it. One adapter
implementation, parameterized by what is committed, serves every state using the capability.

### `mfm-capabilities` — the entry axis

```rust
/// Sealed declaration of whether a parked attempt may be re-entered.
pub trait EffectEntryMode: private::EffectEntryModeSealed + Send + Sync + 'static {
    /// Maximum invocations of one occurrence, including the first.
    const MAX_ENTRIES: u16;
}

/// A repeat is not absorbed. A parked attempt is terminal, exactly as today.
pub enum EntryOnce {}

/// The external system absorbs a repeat of the byte-identical committed request.
pub struct EntryAbsorbing<const MAX: u16>;
```

### `mfm-capabilities` — the obligation

The obligation rides a framework-owned evidence trait whose private marker is parameterized by the
request, mirroring `CapabilitySetFor<E>` (`capabilities/src/lib.rs:560-568, 683, 761`):

```rust
/// Framework-owned evidence that an entry mode is declarable for a request type.
pub trait EffectEntryModeFor<Req>:
    EffectEntryMode + private::EffectEntryModeForSealed<Req>
{
}

impl<Req, M> EffectEntryModeFor<Req> for M where
    M: EffectEntryMode + private::EffectEntryModeForSealed<Req>
{
}

pub trait EffectCapabilityContract: Send + Sync + 'static {
    type Request: MfmValue;
    type Returned: MfmValue;
    type SafeFailure: MfmValue;
    type Refresh: EffectRefreshMode;
    /// Re-entry discipline, declarable as absorbing only over a keyed request.
    type Entry: EffectEntryModeFor<Self::Request>;
}
```

```rust
mod private {
    pub trait EffectEntryModeSealed {}
    pub trait EffectEntryModeForSealed<Req> {}

    impl<Req> EffectEntryModeForSealed<Req> for EntryOnce {}

    // The obligation. Absorption is unavailable without an entry key.
    impl<Req, const MAX: u16> EffectEntryModeForSealed<Req> for EntryAbsorbing<MAX>
    where
        Req: EntryKeyed,
    {
    }
}
```

**Do not** instead parameterize `EffectEntryMode` itself as `EffectEntryMode<Req>` while leaving the
seal unparameterized. That compiles, looks sealed, and is not: orphan rules permit
`impl ForeignTrait<LocalType> for ForeignType`, so a downstream crate writes
`impl EffectEntryMode<LocalRequest> for EntryAbsorbing<3>` over an unkeyed request and it builds
clean. A compile-fail test must pin this exact attack, and it must fail for the *sealing* reason.

`Refresh` is untouched and stays exactly what it is — the live rotation path where an adapter proves
it did not enter and the binding advances. The two axes are independent and EVM declares both.

### `mfm-spec` — one new field on the Effect protocol

```rust
/// Sealed re-entry discipline for one Effect capability.
pub enum StructuredEffectEntryContract {
    /// A parked attempt is terminal.
    EntryOnce {},
    /// A repeat of the byte-identical committed request is absorbed.
    EntryAbsorbing {
        /// Exact retained entry-key contract.
        entry_key_contract_ref: ContentRef,
        /// Maximum authorizations per occurrence, including the first.
        max_entries: NonZeroU16,
    },
}
```

Added to `StructuredCapabilityProtocolContract::Effect` beside `refresh_contract`, not inside
`StructuredEffectRefreshContract` — the axes are orthogonal and nesting them would make
`EntryAbsorbing` unreachable for a `NoRefresh` capability, which is the majority case the design
exists to serve.

`NonZeroU16`, not `u16` with a doc comment: `MAX == 0` is a typed authoring rejection, so the illegal
state never reaches a document. A const generic on the binding cannot express non-zero in Rust, so the
conversion is a runtime check in `contract()` — one place, at authoring time.

### `mfm-store` — two leaves

```rust
pub enum StateLeaf {
    Ready,
    /// One exact access authorization is outstanding. Effect-only after commit 3.
    Authorized { access_attempt_id: AccessAttemptId },
    ObservedForSettlement { access_attempt_id: AccessAttemptId, observation_ref: RecordRef },
    Refreshable { next_attempt_ordinal: u64, public_lineage_head_ref: ContentRef },
    EntryUnknown { access_attempt_id: AccessAttemptId },
    BlockedIntegrity { observation_ref: RecordRef },

    /// A crashed Effect attempt on an absorbing capability with budget
    /// remaining. Runtime must commit its closing observation before any
    /// re-assertion. Authorizes nothing.
    EntryClosable { access_attempt_id: AccessAttemptId },
    /// The parked attempt is resolved and a repeat absorbs. The next ordinal
    /// re-asserts the byte-identical committed request.
    Reassertable { next_attempt_ordinal: u64 },
}
```

**One `Reassertable`, not one per access kind.** Commit 3 introduces it for the crashed Read; commit 5
adds the Effect producers. The leaf carries only an ordinal, and the access kind is in scope at every
site that reads it, so a second variant would carry no information the reader lacks. It also means the
two access kinds cannot silently diverge in the rules that matter: what admits their successors is
written once, as one access-kind-conditional branch, instead of twice.

`Reassertable` carries **no lineage head, and that omission is the whole cost argument.** Both
`minimum_lineage_head_ref` (`fold.rs:4130-4135`) and `previous_physical_binding_ref`
(`fold.rs:4148-4163`) are `Some` only under `StateLeaf::Refreshable` and fall to the `_ => None` arm
otherwise, so the physical-binding verifier takes its existing `(None, None)` path **unchanged**.
There is no strict-descendant rule to relax and no same-binding case to admit. Confirm this by
reading `verify_authorization` before writing commit 5, and if the diff to
`crates/app/src/production_structured.rs` is not empty, stop — that means the leaf leaked a lineage
obligation and the design's central claim is wrong.

`Authorized.access_kind` is deleted in commit 3: `fold.rs:2987` is its sole producer, and after that
commit it emits `Reassertable` for `AccessKind::Read`, leaving the field constant.

### `mfm-store` — the entry contract reader

`state_leaf` (`fold.rs:2971`) needs the capability's entry contract and currently receives only the
attempt maps. Give it `program: &VerifiedProgramData`; the caller already holds
`engine.machine.program()?` (`fold.rs:2651-2667`). `state_stable_resource_lineage_contract_ref`
(`fold.rs:3792-3815`) already does exactly this decode-validate-identity-check walk for the same
state, so factor its preamble into one `state_capability_protocol(program, state)` and read both
contracts through it. Do not add a second copy of that walk.

---

## Commit sequence

Six commits. Commit 0 is the proposal on the branch. Commits 1 and 2 are independent of the mechanism
and of each other; 3 is independent of 4–6; 5 depends on 4; 6 depends on 5.

Commits 1 and 2 are the design doc's *Required alongside the mechanism*. They are ordered first
because they are cheap, relax nothing, and are the only way to observe the later commits working
against a real park — not because they are preliminaries. Commit 2 in particular is part of the
safety argument, not an operational convenience: without it, parks are found late by construction, so
the mechanism's trigger condition selects for exactly the parks whose absorption window has expired.

---

### Commit 0 — `document the effect entry-resolution proposal` (done)

`docs/effect-entry-resolution.md`, the gap entries, this plan, the README index. No code.

Verification: `git diff --check`; check links and command claims. No Rust gate.

---

### Commit 1 — `carry the parked occurrence through the possible-entry barrier`

The fold computes the full identity of the parked occurrence and then discards it.
`StructuredFrontier::PossibleEntry` is a unit variant (`runtime/src/history/cursor.rs:116`), so the
occurrence, path, attempt id, and capability reference are thrown away and the operator is told only
that a run is blocked. Relaxes nothing; the access-audit projection already retains every field.

**Scope**

- `crates/kernel/runtime/src/history/cursor.rs`: `PossibleEntry` carries the subject — occurrence id,
  occurrence path, access attempt id, capability contract ref. `state_frontier` and
  `fan_out_frontier` (`fold.rs:3027-3088`) already hold the `ActionableState`; the change is to stop
  dropping it. Keep the fan-out rejection of `PossibleEntry` (`fold.rs:3073-3077`).
- `crates/kernel/runtime/src/structured.rs`: `DriveOutcome::PossibleEntry` carries the same subject
  (`~476-479`).
- `crates/app/src/surface.rs` (`~754`): the `operational_block` reason gains the subject fields.
  Redaction-safe by construction — every field is already exposed by the access-audit projection.

**Tests**

- Fold: a parked Effect reports the exact occurrence id, path, and attempt id.
- App: the public `DriveResponse` for a parked run names the occurrence, and the serialized shape
  contains no field absent from the audit projection.

**Verification**

```bash
nix develop -c cargo test -p mfm-store -p mfm-runtime -p mfm-app
nix run .#check
```

---

### Commit 2 — `enumerate parked runs`

Every application entry point is per-`RunId`. An operator who does not already hold the run id cannot
find a parked run at all, and every recovery design in the design doc assumes a caller who knows
which run to fix.

**Scope**

- A backend index over parked runs, landing in the SQL-ownership residual. It must be a predicate on
  committed history — an occurrence with a committed authorization and no observation — not a
  materialized status column, so it cannot disagree with the fold.
- One paginated listing surface on `mfm-app`, mirroring the existing per-run pages: reuse
  `PageRequest` (`surface.rs:95-120`) and its cursor/limit discipline verbatim. Tenant-scoped, and
  scoped by nothing else — the point is to find runs nobody is looking for.
- Each row carries commit 1's subject. That is the dependency between these two commits and the only
  one.
- `docs/known-gaps.md`: delete the *No way to find a parked run* entry.

**Tests**

- A run parked at an Effect boundary appears; a healthy run and a closed run do not.
- Pagination is stable across an append, at the existing cursor semantics.
- Scale: the listing plan is indexed and non-aggregate, asserted the way the EVM wallet query tests
  assert theirs.
- Tenant isolation: a parked run in another tenant scope is absent.

**Verification**

```bash
nix develop -c cargo test -p mfm-app -p mfm-storage-postgres
nix run .#test-db
nix run .#check
```

---

### Commit 3 — `recover a crashed read attempt and delete the waiting-reads frontier`

Independent of the entry axis and useful alone: a Read absorbs vacuously, because a Read is already
defined as not consuming externally meaningful state. No closing observation, no declaration, no
budget. The `WaitingReads` deletion is not optional cleanup — after this change its only producer is
gone, and retaining an unreachable frontier is superseded code.

**Scope**

- `crates/kernel/store/src/structured/fold.rs`
  - `state_leaf` (`~2986`):

    ```rust
    let Some(observation) = observations.get(attempt_id) else {
        return Ok(match authorization.record.access_kind {
            // A Read consumes no externally meaningful state, so a lost invoker
            // authority may be reissued at the next ordinal with the unobserved
            // predecessor standing.
            AccessKind::Read => StateLeaf::Reassertable {
                next_attempt_ordinal: authorization
                    .record
                    .attempt_ordinal
                    .checked_add(1)
                    .ok_or_else(|| invalid("access attempt ordinal overflowed"))?,
            },
            AccessKind::Effect => StateLeaf::Authorized {
                access_attempt_id: attempt_id.clone(),
            },
        });
    };
    ```
  - `state_frontier` (`~3027-3051`): `(Read, Reassertable)` actionable. Delete the
    `(Read, Authorized) => WaitingReads` arm and the `StructuredFrontier::WaitingReads` variant.
    Keep the `(Read, Refreshable)` rejection — a Read has no supersession lineage — and add the
    matching `(Read, EntryClosable)` rejection in commit 5. `(Read, Reassertable)` is legal from this
    commit onward — that is the shared leaf.
  - `fan_out_frontier` (`~3053-3088`): delete `saw_waiting` and its trailing branch.
  - Ordinal admission (`~1160`, `~4001-4017`): accept `Reassertable { next_attempt_ordinal }` for
    `AccessKind::Read`.
  - **The one relaxation.** The outstanding-attempt rule (`fold.rs:4018-4029`) must admit an
    unobserved predecessor when the leaf is `Reassertable` and the access kind is `Read`, and must
    keep rejecting in every other case. Write it as an explicit access-kind-conditional branch with
    its own error message, not as a widened predicate.
- `crates/kernel/runtime/src/structured.rs`: delete `DriveOutcome::WaitingReads` and its `drive_once`
  arm (`~471-474`); route `(Read, Reassertable)` into the existing bracket:

  ```rust
  (StructuredExecutionKind::Read, StateLeaf::Ready | StateLeaf::Reassertable { .. }) => {
      self.drive_access::<ReadPhysicalBindingKind>(verified, state, state_identity, input)
          .await
  }
  ```
- Delete the rest of the chain: `StructuredReplayStatus::WaitingReads`
  (`replay/src/structured.rs:184`), `purpose.rs:41,54,65`, and the `retryable_evidence_gap` arm at
  `crates/app/src/surface.rs:751`.
- `docs/design.md`, `docs/run-execution.md`: "an unmatched Read waits" is now wrong; the `drive_once`
  disposition list loses one entry. `docs/known-gaps.md`: delete the Read-strand entry.

**State explicitly in the code comment and in `docs/design.md`:** the fold cannot distinguish a
crashed Read from a live in-flight one, because the leaf is a pure function of history and liveness is
per-process. Two workers may therefore invoke the same Read concurrently. That is admissible under the
Read definition and is the reason this commit is safe — which makes that definition load-bearing where
it previously was not, and it should become a certification obligation rather than prose.

**Tests**

- `store/src/structured/tests.rs`: extend `fresh_folds_resume_every_runtime_crash_boundary` (`~1862`)
  so the post-authorization fold is asserted actionable and a second authorization is admitted. Drop
  the retained in-process authorization handle first — that handle is why this test passes today, and
  a real crash destroys it.
- Fold regression: a second Read authorization at the *same* ordinal is rejected; only the successor
  ordinal is admitted.
- Fold regression: an unobserved *Effect* predecessor still rejects a successor authorization. This is
  the property the relaxation must not reach; assert it explicitly.
- Fold regression: `(Effect, Authorized)` still folds to `PossibleEntry` — unchanged by this commit.
- Runtime: a re-driven run with a durable unobserved Read authorization invokes the registered invoker
  exactly once more and commits one observation. Assert the call count.
- Grep-level assertion that no `WaitingReads` spelling survives anywhere.

**Verification**

```bash
nix develop -c cargo test -p mfm-store -p mfm-runtime -p mfm-replay -p mfm-app
nix run .#check
nix run .#test
```

---

### Commit 4 — `declare the effect entry mode`

Type-level and certified contract. No behaviour change: every existing capability declares
`EntryOnce`, and the new certified field is inert until commit 5. This commit is where the compile-time
force lives, and it is worth landing even if the gate rejects commit 5.

**Scope**

- `crates/kernel/capabilities/src/lib.rs`: `EntryKeyed`, `EffectEntryMode`, `EntryOnce`,
  `EntryAbsorbing<MAX>`, `EffectEntryModeFor`, the parameterized private seals, and the
  `EffectCapabilityContract::Entry` associated type — all as in *Reference types*.
- `crates/kernel/spec/src/structured.rs` (`~886-905`): `StructuredEffectEntryContract` and the new
  field on `StructuredCapabilityProtocolContract::Effect`, plus accessors and `validate()` coverage.
- `crates/kernel/program/src/structured.rs`: a `RuntimeEffectEntryBinding<Mode>` sealed the way
  `RuntimeEffectRefreshBinding` is (`program/src/structured.rs:160, 476-497`), emitting the certified
  contract. Resolve the entry-key contract ref in `runtime_effect_capability_contract`, which already
  has `Capability::Request` in scope — do **not** thread the request type through the binding
  generics. `NonZeroU16::new(MAX)` failing is a `ProgramError::Authoring`.
- `crates/kernel/certify/src/structured.rs` (`~2568-2586`): register the entry-key contract as an
  outbound reference beside the refresh evidence contract.
- Every existing Effect capability declares `type Entry = EntryOnce;`. Mechanical, and the compiler
  enumerates the sites.

**Tests**

- Compile-fail (trybuild), in this order:
  1. the orphan attack — a downstream `impl EffectEntryMode<LocalReq> for EntryAbsorbing<3>` over an
     unkeyed request — fails, and fails for the sealing reason. **Write this first.** It is the test
     that distinguishes a real seal from one that only looks like one.
  2. `type Entry = EntryAbsorbing<3>` over a request that does not implement `EntryKeyed` fails.
  3. a request whose key field is `Option<_>` cannot implement `EntryKeyed` without an unreachable
     branch. This is the autoincrement case, and it must fail at the type level rather than park at
     runtime.
- Authoring rejection: `EntryAbsorbing<0>` fails `contract()` with an authoring error.
- Unit: `entry_key` is deterministic across calls.
- Golden update: certification goldens carrying an Effect capability protocol.

**Worked non-EVM fixtures, in the kernel test corpus.** EVM's key is minted by a reservation
authority, which is the easy shape. Add a fixture capability whose key must be caller-supplied — an
`InsertRow` Effect with `Request = InsertRequest { table, columns, values, idempotency_key }` — and
its autoincrement sibling with no key, which must fail to declare `EntryAbsorbing`. The two together
pin both halves of the contract, and neither is exercised by the EVM path. Add a third whose
`Returned` is `RowsAffected(u64)` and record in its rustdoc that it is the shape the
post-state-functional rule forbids; nothing can check that, and the fixture is the closest thing to a
check that exists.

**On uniqueness — do not add a kernel seed.** A key derived from the request is not unique per
occurrence in general. The repository already answers this and its answer is better: the disambiguator
is the caller submission token, which flows through the state input into `derive_submission_intent_id`
(`wallet_authority.rs:508-521`) and seeds every operation key. Those keys are deliberately cross-run
stable, which is what makes every surviving reconciliation construct work; a seed containing `run_id`
destroys that by construction. Where an occurrence-unique external key is genuinely needed, the kernel
already mints one after the authorization is durable and before entry:
`CertifiedAccessAuthorization::access_attempt_id()` is handed to the invoker and the same bytes are in
the committed record. Threading a seed into `author_request` instead would touch ~17 call sites and
four erased layers in `mfm-certify`, and would make certification-time sample authorship diverge from
production.

**Verification**

```bash
nix develop -c cargo test -p mfm-capabilities -p mfm-spec -p mfm-program -p mfm-certify
nix run .#check
nix run .#test
```

---

### Commit 5 — `close and re-assert a parked effect attempt`

The behavioural cutover, and the commit the gate governs. Fold and Runtime are inseparable.

**Fold** (`crates/kernel/store/src/structured/fold.rs`)

`state_leaf` gains `program: &VerifiedProgramData` and reads the entry contract through the shared
`state_capability_protocol` helper from *Reference types*. Both parked shapes route through one
function, and they land on **different** leaves:

```rust
fn parked_effect_leaf(
    entry: &StructuredEffectEntryContract,
    attempt_id: &AccessAttemptId,
    attempt_ordinal: u64,
    observed_entry_unknown: bool,
) -> super::Result<StateLeaf> {
    // Ordinals are dense per occurrence, so the ordinal is the attempt count and
    // the budget needs no extra folded state.
    let budget_remaining = match entry {
        StructuredEffectEntryContract::EntryAbsorbing { max_entries, .. } => {
            attempt_ordinal.saturating_add(1) < u64::from(max_entries.get())
        }
        StructuredEffectEntryContract::EntryOnce {} => false,
    };
    if !budget_remaining {
        // No declared absorption, or the budget is spent: the occurrence parks
        // exactly as today. This is the floor, not a failure of the design.
        return Ok(if observed_entry_unknown {
            StateLeaf::EntryUnknown { access_attempt_id: attempt_id.clone() }
        } else {
            StateLeaf::Authorized { access_attempt_id: attempt_id.clone() }
        });
    }
    Ok(if observed_entry_unknown {
        StateLeaf::Reassertable {
            next_attempt_ordinal: attempt_ordinal
                .checked_add(1)
                .ok_or_else(|| invalid("access attempt ordinal overflowed"))?,
        }
    } else {
        // Authorized and unobserved: the invoker authority is lost and the
        // attempt must be closed before anything re-asserts.
        StateLeaf::EntryClosable { access_attempt_id: attempt_id.clone() }
    })
}
```

`state_frontier` (`~3027-3051`) — `(Effect, Reassertable)` joins the existing actionable Effect arm,
plus one added arm and one rejection. No relaxed equalities:

```rust
(StructuredExecutionKind::Effect, StateLeaf::EntryClosable { .. }) => {
    StructuredFrontier::Actions(vec![state.clone()])
}
(StructuredExecutionKind::Read, StateLeaf::EntryClosable { .. }) => {
    return Err(invalid("Read state cannot be entry-closable"));
}
```

Ordinal admission (`~1160`, `~4001-4017`) — `Reassertable` gains its Effect arm beside the Read arm
commit 3 added. `EntryClosable` authorizes nothing and must reach the `_ =>` rejection arm; assert
that.

```rust
let attempt_ordinal = match (&actionable.leaf, access_kind) {
    (StateLeaf::Ready, _) => 0,
    (StateLeaf::Refreshable { next_attempt_ordinal, .. }, AccessKind::Effect)
    | (StateLeaf::Reassertable { next_attempt_ordinal }, _) => *next_attempt_ordinal,
    _ => return Err(invalid("current state leaf cannot authorize access")),
};
```

The `Reassertable` arm is access-kind-agnostic here on purpose: the ordinal is the ordinal either way.
What differs between the kinds is the outstanding-attempt rule, and that stays in exactly one place —
the access-kind-conditional branch commit 3 wrote.

**The new check the fold does not have today.** A re-assertion under `Reassertable` must carry
`occurrence_id`, `state_input_ref`, `request.contract_ref`, and `request_digest` byte-equal to its
predecessor (`journal/src/structured.rs:1026, 1032, 1056, 1058`). It costs nothing and is free of
trust: `author_request` is already a deterministic function of state input with no ambient IO, and the
re-asserting attempt has the same `state_input_ref`, so Runtime re-authors and gets identical bytes
with no new request path. This turns that determinism invariant into a *verified* property exactly
where a violation would be a duplicate effect with different content. Write it as its own function
with its own error per field.

**What must stay untouched, asserted by reading the diff:**

- the outstanding-attempt rule (`fold.rs:4018-4029`) — the Effect path keeps it verbatim, and the
  closing observation is why it can. An earlier revision of the design relaxed it instead — permit one
  unobserved predecessor when the leaf is re-assertable — and that variant strands the runs it claims
  to resolve: crash, re-assert, crash again leaves two unobserved predecessors and the third attempt
  is rejected; crash, re-assert, observe `EntryUnknown` leaves the unobserved predecessor no longer
  immediate and the third is rejected on either reading. Do not reintroduce it.
- `crates/app/src/production_structured.rs` — **empty diff**, per *Reference types*.
- the refresh-predecessor-has-an-observation check (`fold.rs:4148-4163`) — unchanged, because it is
  gated on `StateLeaf::Refreshable` and a `Reassertable` predecessor is observed by construction.
- every equality: `fold.rs:3996` (access kind equals the certified execution kind), `fold.rs:4411`
  (supersession requires `Refreshable`), `fold.rs:4441` (`EntryUnknown` requires Effect),
  settlement linkage. `attempts.last()` keeps its meaning at both reader sites (`fold.rs:2979`,
  `fold.rs:4151`) because there is one re-assertion role and one ordinal space.

**Runtime** (`crates/kernel/runtime/src/structured.rs`)

Re-assertion is one added pattern on the existing bracket (`~582`):

```rust
(
    StructuredExecutionKind::Effect,
    StateLeaf::Ready | StateLeaf::Refreshable { .. } | StateLeaf::Reassertable { .. },
) => {
    self.drive_access::<EffectPhysicalBindingKind>(verified, state, state_identity, input)
        .await
}
```

No new `DriveOutcome` variant, no `RuntimeFaultPhase` variant, no adapter change: `K::EFFECT` is true,
so `observation_outcome` admits `SupersededBeforeEntry` and `EntryUnknown` unchanged
(`runtime/src/structured.rs:1162-1191`). A genuinely indeterminate re-assertion returns `EntryUnknown`
and folds back to `Reassertable` while budget remains.

Closing is a new, short path — and it is short because the observation pipeline never needed an
in-process authorization handle. `qualify_invoked_observation` and `commit_pending_observation`
consume only `authorization_ref`, `access_attempt_id`, `outcome`, and the verified run, and they
resolve the authorization out of folded history via
`VerifiedRunView::authorization(&AccessAttemptId) -> Option<(&RecordRef, &ExternalAccessAuthorized)>`
(`runtime/src/history/port.rs:45-48`). So:

```rust
(StructuredExecutionKind::Effect, StateLeaf::EntryClosable { access_attempt_id }) => {
    self.close_parked_attempt(verified, state, access_attempt_id).await
}
```

`close_parked_attempt` looks up `(authorization_ref, _)` from `verified`, builds
`ProposedObservationOutcome::EntryUnknown { fault_code: INVOKER_AUTHORITY_LOST }`, and hands it to the
existing `qualify_invoked_observation` / `commit_pending_observation` pair, returning
`DriveOutcome::AccessObserved`. It authors nothing, invokes nothing, and reaches no adapter. Amend the
`AccessObserved` doc comment (`~274-275`), which currently says "invoked exactly once".

`ExternalAccessObserved` carries only `authorization_ref`, `access_attempt_id`, and `outcome`
(`journal/src/structured.rs:1101-1108`), so the closing record is shape-identical to an
adapter-reported one and the reserved fault code is the only discriminator. `adapter_origin` is
Runtime-internal fault attribution only; pass the capability identity.

**Why the closing observation is honest, and must be stated in its rustdoc.** It asserts nothing about
the external system. It asserts that the invoker authority for this attempt is lost and entry is
unknown — the literal truth of a crash. Observation admission already requires exactly an existing
authorization with no prior observation, so the record is legal history today. This is the one
observation Runtime can synthesize without invoking anything and without lying, and if a future edit
makes Runtime synthesize any other outcome, that property is gone.

**Races, all closed by existing rules — assert each.** Two workers may race the closing observation,
or close a slow attempt that is still live. The first commit wins and the second is rejected by the
one-observation-per-attempt rule. A live invoker whose attempt was closed under it delivers its real
completion late, and that too is rejected as a second observation; whatever it did to the external
system is absorbed by the re-assertion at the next ordinal. That is the same collapse absorption
already declares harmless, not a new one — and it is why closing is scoped to `EntryAbsorbing`, so an
`EntryOnce` park is never touched.

**One thing leaves the trust boundary, and it is this design's main safety gain.** Nothing on this
path asserts non-entry. `SupersededBeforeEntry` is the only completion whose wrongness silently
duplicates, and this path neither produces nor consumes it. Safety rests on a positive claim about the
external system, made once, at the capability. Do not let a later commit mint `SupersededBeforeEntry`
from "the row isn't there".

**Docs**: `docs/design.md` *Runtime access choke point*; `docs/run-execution.md` completion table;
`docs/effect-entry-resolution.md` status becomes the current contract, with the two-leaf correction
and the Read relaxation folded into *Behaviour*; `docs/known-gaps.md` rewritten per the entries below.

**Tests**

Fold, hostile-history focus — persisted input is hostile, so every one of these is a store-side
rejection test, not a type-level one:

- a parked attempt on an `EntryOnce` capability still folds to `PossibleEntry`, for both park shapes;
- a re-assertion when the budget is spent is rejected;
- a re-assertion at the wrong ordinal is rejected;
- a re-assertion whose `request_digest` differs is rejected; likewise `state_input_ref`,
  `request.contract_ref`, and `occurrence_id` — four separate tests, four distinct errors;
- a *second concurrent* re-assertion is rejected — the outstanding-attempt rule is unchanged on the
  Effect path and must stay unchanged; assert it explicitly, because this is the property the earlier
  revision lost;
- an `EntryClosable` leaf rejects an authorization at any ordinal;
- crash → close → re-assert → crash → close → re-assert reaches the third attempt and stops at
  `MAX_ENTRIES`. This is the exact sequence the rejected relaxation stranded; it is the regression
  that proves closing was the right fix;
- a second observation on a closed attempt is rejected, including one carrying a real `Returned`;
- a `SupersededBeforeEntry` observation on a capability certified `NoRefresh` is still rejected at the
  fold (`fold.rs:4411-4425`), with the rejection reason asserted to be the protocol gate rather than
  Rust uninhabitedness.

Runtime:

- a crash-shaped history with a parked absorbing Effect closes, re-invokes the adapter exactly once,
  and settles through the ordinary callbacks;
- a committed `EntryUnknown` re-asserts until the budget is spent, then reports `PossibleEntry`
  carrying commit 1's subject;
- **total adapter invocations for one occurrence never exceed `MAX_ENTRIES`.** This is the bound the
  whole design rests on; assert the count, not the outcome;
- closing invokes no adapter at all — assert a zero call count on the closing drive specifically;
- a `SupersededBeforeEntry` return still advances the ordinal and re-invokes exactly once, proving the
  two axes are independent.

Replay, audit, export: a run resolved by re-assertion replays callback-free to the same head; the
audit projection shows every attempt at its ordinal and distinguishes the synthesized closure from an
adapter-reported `EntryUnknown`; a history containing a closure and a re-assertion round-trips.

**Verification**

```bash
nix develop -c cargo test -p mfm-store
nix develop -c cargo test -p mfm-runtime -p mfm-replay -p mfm-app
nix run .#ci
```

Do not run the component gates immediately before `.#ci` on the same revision. Confirm
`recoverability-postgres-v1` passes inside `.#ci`: records and identity preimages are unchanged, so no
annex change is expected, and a failure means that assumption was wrong.

---

### Commit 6 — `resolve evm wallet and broadcast entry by absorption`

Declares `EntryAbsorbing` on the four EVM capabilities and makes their adapters re-entrant. All four
already carry a key, so no request type changes shape.

**Deletions: none.** See *What is removed from EVM* above. If the integration test shows
`ActivateCandidateResponse::AlreadyRetained` has become unreachable, delete that settlement branch in
this commit.

**Scope**

- Four `EntryKeyed` impls: `ReserveEvmNonceRequest.reservation_key`,
  `ActivateEvmCandidateRequest.candidate_operation_key`, `CompleteEvmNonceRequest.completion_key`, and
  `BroadcastExactCandidateRequest.active_candidate.attested_candidate.transaction_hash`.

  ```rust
  impl EntryKeyed for ReserveEvmNonceRequest {
      type EntryKey = EvmNonceReservationKey;

      fn entry_key(&self) -> Self::EntryKey {
          self.reservation_key.clone()
      }
  }
  ```
- `crates/domains/evm/src/wallet_authority.rs` (`~3116`) and `submission.rs` (`~918`): each capability
  declares `type Entry = EntryAbsorbing<3>;` and its binding. Each rustdoc states *what absorbs* — the
  unique constraint or nonce slot the claim rests on — and that its `Returned` is a function of
  authority post-state, not of one exchange. That sentence is the entire unverifiable half of the
  contract; make it hard to delete accidentally.
- `crates/storages/evm-postgres/src/authority.rs`: the three wallet adapters recognize their own prior
  write on re-invocation, keyed by the request's entry key:

  | Authority state | Completion |
  | --- | --- |
  | this exact operation key is present | `Returned(<the effect's own response>)` |
  | absent and the writer lease is fenced | `SupersededBeforeEntry(WalletNonceStoreLineageHead)` |
  | absent, unfenced, or backend unavailable | `EntryUnknown(fault)` |
  | present with conflicting content | `IntegrityFault(fault)` |

  The second row is the dangerous one and belongs to the `Refresh` axis, not this one. Today
  `SupersededBeforeEntry` is minted only from real fencing — `cache_supersession` requires
  `evidence.writer_epoch > store_incarnation.writer_epoch` (`authority.rs:536-545`). Do **not** mint
  it from "the row isn't there": `begin_write` runs at `READ COMMITTED` (`authority.rs:608`), so a
  parked writer's uncommitted transaction is legitimately invisible and can commit afterwards.
  Absent-and-unfenced is `EntryUnknown`, not non-entry.
- Every re-entrant adapter reads first and enters only on the path that would enter anyway, so write
  authority is exercised only when a write is actually required. This is an implementation obligation,
  not a contract; the restart tests below are what hold it.
- `crates/live/evm/src/structured.rs`: the broadcast adapter, re-invoked, determines whether the
  attested transaction entered. **It must read the chain.** An earlier draft forbade that on the
  grounds that `ObserveActivatedTransactionState` already reads the chain; that was wrong. Those are
  authored program Reads on the settled path. Without a chain read the adapter can only see that the
  candidate is still activated — which is exactly the state in which the broadcast parked *without*
  entering — and `SubmittedCandidateProof` (`submission.rs:722-737`) is entirely derivable from the
  attested candidate, so it would synthesize a well-typed proof of an event that never happened. The
  run would then wait forever for a receipt with the nonce occupied by a transaction never sent: a
  silent wedge, strictly worse than today's visible park.
- `docs/evm-transactions.md` (`~164`): `EntryUnknown` no longer parks indefinitely.

**Tests**

- `crates/storages/evm-postgres/tests/wallet_authority.rs`: each row of the mapping table, asserted
  directly. Include a test that an uncommitted concurrent writer yields `EntryUnknown` and never
  `SupersededBeforeEntry`.
- `tests/integration/tests/evm_postgres_submission.rs`: kill the process between authorization and
  observation at the broadcast boundary, restart, drive to completion in the **same** run, and assert
  exactly one transaction reached the provider across both processes. This is the test that proves
  recovery was not bought with the safety half; without it, commit 5 is unverified in production
  shape.
- The same restart at the reserve, activate, and complete boundaries, asserting one authority write
  each.
- A restart at a boundary where the absorbing repeat *does* re-enter — kill after the write commits
  but before the observation — asserting one durable row and one settled outcome.
- Regression: `ObserveRetained` still resolves an older activated replacement that wins after a newer
  candidate was activated. This construct is deliberately kept and must not regress.

**Verification**

```bash
nix develop -c cargo test -p mfm-evm -p mfm-evm-live -p mfm-storage-evm-postgres
nix run .#test-db
nix run .#run -- --task evm-postgres-submission-qualification
```

Run `.#ci` once on the final revision.

---

## Acceptance

- an `EntryOnce` capability behaves exactly as today, for both park shapes, and the declaration is
  certified and reportable before deployment;
- an `EntryAbsorbing` capability resolves both park shapes — crashed and observed `EntryUnknown` — by
  closing and re-asserting, or reports `PossibleEntry` when the budget is spent;
- total adapter invocations per occurrence never exceed `MAX_ENTRIES`, asserted by call count;
- a capability cannot declare `EntryAbsorbing` over a request with no entry key, and the downstream
  orphan attack fails to compile for the sealing reason;
- a re-assertion with any differing request field is rejected at the fold;
- the outstanding-attempt rule is unchanged on the Effect path, and relaxed on the Read path only,
  with a regression pinning each half;
- `crates/app/src/production_structured.rs` is unmodified across the whole plan;
- a crashed Read occurrence recovers, and no `WaitingReads` spelling survives anywhere;
- a parked run is discoverable without prior knowledge of its run id, and names its occurrence;
- all four EVM Effect boundaries resolve in-run, each with a restart test asserting one external
  write;
- five record families, `ObservationOutcome` unchanged, `ExternalAccessAuthorized` unchanged,
  `AccessKind` unchanged, and every `access_kind == Effect` guard still an equality.

## Out of scope

- The seven-state / one-capability duplication in the EVM status reads.
- Generalizing the hard-coded resource dispatch in `verify_supersession`.
- The absent append-only witness.
- Any time bound on absorption. The fold has no clock and a TTL field would read as a guarantee it
  cannot enforce; see *Durable versus windowed absorption* in the design doc.
- Any claim that the external system absorbs a repeat, that an adapter transmits the entry key, that
  absorption is retained long enough, or that `Returned` is post-state-functional. All four are
  declared by naming `EntryAbsorbing<MAX>` and none is verifiable.
