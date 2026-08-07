# Effect entry resolution

Status: proposed design contract, not implemented, direction not yet accepted

An Effect occurrence parks forever when a process dies between committing an authorization and
committing an observation. This document proposes the mechanism that resolves it.

Two earlier revisions are recorded in *Rejected alternatives* with the reasons they failed, and one
adjacent design is recorded there as deferred rather than failed. All are plausible enough to be
reproposed otherwise.

## Material uncertainties

- **The uniqueness trade is unresolved and is not a technical question.** Today "an Effect occurrence
  reaches the external system at most once, ever, regardless of adapter correctness" is structural,
  enforced callback-free on every refold. This design replaces it with "at most `MAX` times, and
  repeats absorb by declaration". Whether that trade is acceptable is a deployment judgment. The
  alternative that keeps the structural guarantee is *Authored recovery routes*, below; *Positive
  adoption by read* also keeps it, and recovers only the entered half of the park space.
- **The fold has no clock and cannot have one.** Much real-world absorption is time-bounded: an
  idempotency key expires, a dedup window closes. The budget bounds the number of attempts, never
  the interval between them, and the fold cannot be made to — a wall-clock-conditioned fold breaks
  refold equivalence. See *Durable versus windowed absorption*.
- **`Returned` must be post-state-functional, and nothing checks it.** See that section.

## The problem

An Effect occurrence parks when the run holds a committed authorization with no observation. Two
boundaries produce it: the process died between `ExternalAccessAuthorized` and
`ExternalAccessObserved`, or the invoker returned `EntryUnknown`.

The safety half is generic and enforced: the effect ran only after its authorization was durable, a
second authorization for the same occurrence is rejected, and the parked state is never actionable.

Recovery is not. The park is terminal, and only the EVM domain resolves it, on a fresh run.

## Why the earlier attempts were not general

Both asked the wrong question. *Did the effect enter?* is unanswerable in a way that matters: a
negative observation is not stable, because an in-flight entry can land immediately after it.

- A separate read-only **probe** answered it with a read. A negative read is exactly the unstable
  thing.
- Re-invocation gated on **`Refreshable`** answered it by making a straggler impossible to land — an
  exclusive monotonic slot. That is correct, and it is why EVM works, but `Refreshable` is
  writer-fencing for *physical binding rotation*: declaring it requires a `RuntimeResourceAuthority`,
  a resource lineage contract, a public lineage head, and store-side supersession verification. A
  Postgres table with a unique index has none of those and needs none of them. Routing the general
  case through that axis makes every capability pay EVM's price.

## What is actually declared: absorption

The right question is not whether the effect entered. It is whether the world in which it entered
and the world in which it did not are **observationally identical after a repeat**.

They are, when the operation absorbs: applying it twice equals applying it once. Then a retry is not
a bet on a negative read — it is a re-assertion that is safe in both worlds, and a straggler from
the first attempt lands as a no-op.

The enabling fact is already true here and was not being used: **the request is content-addressed
and committed before entry, so every attempt of one occurrence necessarily issues byte-identical
bytes.** The only concurrency that can arise is an operation racing a copy of itself. Absorption is
therefore sufficient — no key retention, no exclusive slot, no lease, no fence, and no negative read
at any point.

Absorption has many sources and the kernel needs to know none of them: a unique constraint or
`ON CONFLICT`, a server-honoured `Idempotency-Key`, a `PUT` to a chosen address, a set-to-value, an
account nonce. Which one an author relies on is the adapter's business and the author's
justification. The kernel needs exactly two things: *does a repeat absorb*, and *how many repeats*.

## Contract

One sealed axis, orthogonal to `Refresh`. `Refresh` stays exactly what it is — the live rotation path
where an adapter proves it did not enter and the binding advances — and EVM keeps it. The two are
independent: EVM wants both.

```rust
/// A request that carries the exact value the external system keys this effect on.
///
/// Total by construction: a request cannot carry `key: Option<K>` and implement
/// this honestly, so declaring absorption forces the field to be mandatory in
/// the request shape. A capability over a target with no such value cannot
/// declare absorption — a compile error, not a runtime strand.
pub trait EntryKeyed: MfmValue {
    type EntryKey: MfmValue + Eq;

    fn entry_key(&self) -> Self::EntryKey;
}

/// Sealed declaration of whether a parked attempt may be re-entered.
pub trait EffectEntryMode: private::EffectEntryModeSealed + Send + Sync + 'static {
    /// Maximum invocations of one occurrence, including the first.
    const MAX_ENTRIES: u16;
}

/// A repeat is not absorbed. A parked attempt is terminal, exactly as today.
pub enum EntryOnce {}

/// The external system absorbs a repeat of the byte-identical committed request.
pub struct EntryAbsorbing<const MAX: u16>;

/// Framework-owned evidence that an entry mode is declarable for a request type.
pub trait EffectEntryModeFor<Req>:
    EffectEntryMode + private::EffectEntryModeForSealed<Req>
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

Sealing the *parameterized* marker is load-bearing. Sealing the unparameterized mode and taking the
request as a trait parameter compiles, looks sealed, and is not: orphan rules permit
`impl ForeignTrait<LocalType> for ForeignType`, so a downstream crate declares absorption over an
unkeyed request and it builds clean. A compile-fail test must pin that exact attack.

The certified Effect protocol gains one field:

```rust
pub enum StructuredEffectEntryContract {
    EntryOnce {},
    EntryAbsorbing {
        /// Exact retained entry-key contract.
        entry_key_contract_ref: ContentRef,
        max_entries: NonZeroU16,
    },
}
```

`MAX == 0` is a typed authoring rejection, so the illegal state never reaches a document.

The field goes on the Effect protocol beside `refresh_contract`, not inside it. Nesting the two would
make absorption unreachable for a `NoRefresh` capability — which is the majority case this design
exists to serve.

## Behaviour

Two new leaves, because the two park shapes are not interchangeable:

```rust
    /// A crashed attempt on an absorbing capability with budget remaining.
    /// Its closing observation must commit before anything re-asserts.
    /// Authorizes nothing.
    EntryClosable { access_attempt_id: AccessAttemptId },
    /// The prior attempt is closed and a repeat absorbs. The next ordinal
    /// re-asserts the byte-identical committed request.
    Reassertable { next_attempt_ordinal: u64 },
```

One leaf does not suffice. A leaf that already says *re-assert* gives Runtime no instruction to close
first, and closing is what leaves the outstanding-attempt rule alone — so the crashed shape needs its
own leaf, distinct from the closed one, and it must reach the ordinal-admission rejection arm like any
other non-authorizing leaf.

`Reassertable` carries no lineage head, and that omission is why this design is cheap:
`minimum_lineage_head_ref` and `previous_physical_binding_ref` are `Some` only under the `Refreshable`
leaf and stay `None` here, so the physical-binding verifier takes its existing `(None, None)` arm
**unchanged**. There is no strict-descendant check to relax and no same-binding case to admit.

`Reassertable` derives only from an attempt **observed** `EntryUnknown`, on a capability declaring
`EntryAbsorbing`, while the ordinal is below the budget. `EntryClosable` derives from an attempt
authorized and unobserved under the same two conditions. Every other parked shape folds exactly as
today, to a terminal `PossibleEntry`.

A crashed attempt is therefore first **closed**: Runtime commits an `EntryUnknown` observation against
the dead attempt's authorization. Observation admission already requires exactly an existing
authorization with no prior observation, and it validates nothing about an `EntryUnknown` beyond
`access_kind == Effect`, so the record is legal history today with no contract change. It is the one
observation Runtime can synthesize without invoking anything and without lying: it asserts nothing
about the external system, only that the invoker authority for this attempt is lost and entry is
unknown — the literal truth of a crash. Both park shapes converge on one closed form before any
re-assertion, and every attempt in a *resolved* history carries a terminal observation instead of a
dangling authorization.

Closing reaches no adapter, authors no request, and needs no in-process authorization handle: the
observation pipeline resolves the authorization out of folded history, so the record the crash
destroyed is not required to close it.

One consequence must not be lost. A committed observation carries `authorization_ref`,
`access_attempt_id`, and `outcome` and nothing else — no origin — so a synthesized closure is
shape-identical to an adapter-reported ambiguity. The fault code is the **only** discriminator
anywhere downstream. It must be one kernel-owned code, unreachable from any adapter, and the audit
projection must distinguish the two.

Closing is what keeps the single-outstanding-attempt rule **unchanged on the Effect path**: a
successor is authorized only when every predecessor is observed, exactly as today. An earlier revision
of this section instead relaxed that rule — permit exactly one unobserved predecessor when the leaf is
`Reassertable`, and only the immediate ordinal predecessor — and that variant strands the runs it
claims to resolve. Crash, re-assert, crash again leaves two unobserved predecessors, and the third
attempt is rejected. Crash, re-assert, observe `EntryUnknown` leaves the unobserved predecessor no
longer immediate, and the third attempt is rejected on either reading of the rule. Terminal park
with budget unspent, and the `EntryUnknown` retry loop unreachable after any crash. The closing
observation does not repair the relaxation; it makes it unnecessary.

Three fold changes:

1. An attempt observed `EntryUnknown`, under a declared `EntryAbsorbing`, with budget remaining,
   derives `Reassertable { next_attempt_ordinal }` rather than a terminal leaf, and ordinal
   admission accepts exactly that ordinal. Ordinals stay dense per occurrence, so the ordinal is the
   attempt count and the budget needs no extra folded state.
2. An attempt authorized and unobserved, under the same two conditions, derives `EntryClosable`
   rather than a terminal leaf. Reading the declaration means the leaf derivation needs the certified
   capability protocol, which it does not take today.
3. A new check the fold does not have today: a re-assertion must carry `occurrence_id`,
   `state_input_ref`, request contract ref, and `request_digest` byte-equal to its predecessor.

The third costs nothing and is free of trust. `author_request` is already a deterministic function
of state input with no ambient IO, and the re-asserting attempt has the same `state_input_ref`, so
Runtime re-authors and gets identical bytes with no new request path. The fold turns that
determinism invariant into a *verified* property exactly where a violation would be a duplicate
effect with different content.

The adapter is unchanged: no new trait, no new method, no new completion variant. The existing
five-variant algebra already covers everything — a genuinely indeterminate retry returns
`EntryUnknown` and folds back to `Reassertable` while budget remains, landing in the same closed
form as a crash.

Two workers may race the closing observation, or close a slow attempt that is still live. The first
commit wins and the second is rejected by the one-observation-per-attempt rule. A live invoker whose
attempt was closed under it delivers its real completion late, and that too is rejected as a second
observation; whatever it did to the external system is absorbed by the re-assertion at the next
ordinal. That is the same collapse absorption already declares harmless, not a new one — and it is
why closing is scoped: Runtime closes a crashed attempt only when the capability declares
`EntryAbsorbing`, so the closure has a consumer, and an `EntryOnce` park is untouched.

Everything else holds: five record families, `ObservationOutcome` unchanged,
`ExternalAccessAuthorized` unchanged, `ExternalAccessObserved` unchanged, `AccessKind` unchanged, the
single-outstanding-attempt rule unchanged on the Effect path, the physical-binding verifier unchanged,
every `access_kind == Effect` guard still an equality, one re-assertion role, one ordinal space,
`attempts.last()` keeps its meaning, and Runtime's bracket for driving an access unchanged — the only
new Runtime path is the one that commits a closure.

## Read recovery falls out

A crash between authorize and observe strands a `Read` occurrence identically. It needs no separate
mechanism, because **a Read absorbs vacuously**: a Read is already defined as not consuming
externally meaningful state, so a repeat is a no-op by that definition. A Read declares nothing, has
no budget, and is never closed first — `EntryUnknown` is an entry outcome, and a Read has no entry to
be unknown about — so an unobserved Read attempt folds straight to `Reassertable`. That is the same
leaf, not a parallel one: the leaf carries only an ordinal, and the access kind is already in scope
everywhere it is read.

**This is where the design does relax an invariant, and it is the only place.** The
single-outstanding-attempt rule is written once, unconditionally across access kinds: a successor is
rejected while any predecessor holds an authorization with no observation. Admitting the Read
successor with its predecessor standing is that rule relaxed, scoped to `Read`. It is sound precisely
because a repeat consumes nothing — and for an Effect the same admission is exactly what the closing
observation exists to avoid, which is why the relaxation must be written as an access-kind-conditional
branch with its own rejection message rather than a widened predicate. Getting this wrong silently
grants the Effect path the relaxation whose failure mode is recorded above.

So Read and Effect recovery share one leaf and one re-assertion path, and differ in two ways, not one:
whether the parked attempt must first be closed, and which rule admits its successor.

`StructuredFrontier::WaitingReads` dies with it: `(Read, Authorized)` is its only producer. The
frontier variant, the drive outcome, the replay status, the purpose projection, and the
`retryable_evidence_gap` app arm are all unreachable afterwards and must be deleted, not retained.

The fold cannot distinguish a crashed attempt from a live in-flight one — the leaf is a pure function
of history and liveness is per-process. Two workers may therefore invoke concurrently. That is what
absorption asserts is harmless, and it is the same collapse, not a different one. It also makes the
Read purity definition load-bearing where it previously was not, and it should become a certification
obligation rather than prose.

## `Returned` must be post-state-functional

Declaring `EntryAbsorbing` asserts that `Returned` and `SafeFailure` are functions of the external
system's **post-state**, not of the response that happened to arrive.

`RowsAffected(1)` violates it — that is a property of one exchange. `InsertOutcome { key, row }`
satisfies it. `PaymentIntent { id, status, amount }` satisfies it; `HttpStatus(201)` does not. The
409-versus-201 and "0 rows"-versus-"1 row" asymmetries then stop being settlement problems, because
neither shape is what `Returned` names: the adapter maps both onto the one post-state value, which
is work it already does on the first attempt.

Widening `Returned` to `Fresh(T) | Deduped(D)` is the wrong move. It pushes the attempt number into
the settlement callback, so every state using the capability branches on whether the run once
crashed — reproducing the per-site obligation this design removes, and making a resolved occurrence
distinguishable downstream from one that never parked.

Nothing checks this rule. It is checkable by review, not by types.

## What can never be resolved

Three cases, and they are one case wearing three costumes — the effect has no name for what it did:

- **No key exists.** A Postgres insert with an autoincrement primary key and no natural key; an HTTP
  POST against a server with no idempotency support. Nothing distinguishes this effect from an
  identical one someone else made.
- **The response is once-only.** A one-shot token, an issued API key, a queue receive that deletes.
  A repeat is safe and recovers nothing, because the deduped response cannot produce an inhabitant
  of `Returned`. An adapter that fabricates one commits a lie into history.
- **Absorption is real but windowed.** See below. This is the worst of the three, because it looks
  resolvable.

For the first two, `EntryOnce` is the whole honest answer, and it is not a weak one. Today the strand
is discovered after a crash by an operator reading a status string. Under this design it is a
certified, per-capability, compile-time-forced property, visible in the program document and
reportable before deployment. The gap does not close; it becomes **declared, bounded, and
enumerable**.

## Durable versus windowed absorption

Absorption backed by durable state — a unique index, an object address, a nonce slot — does not
expire. Absorption backed by a retention window — an idempotency key with a TTL, a dedup window —
does, and a run parked past the window is re-entered without absorption.

The fold cannot see this. Run history carries no timestamps and the fold has no clock, by
construction: a wall-clock-conditioned fold breaks refold equivalence. A TTL field is therefore
deliberately **not** in the contract — an unenforceable declaration is worse than an absent one,
because it reads as a guarantee.

Prefer state-backed absorption. Where only a window is available, its length is a deployment
property, and it belongs in the same place as "do not restore the database from backup". The time to
*find* a park is as material as the window's length, which is why *Enumerate parked runs*, below, is
required rather than adjacent.

## Trust boundary

The store verifies, callback-free, on hostile bytes: that the capability declared absorption; that
the ordinal is exactly the folded successor; that the budget is not exceeded; that the re-asserted
request is byte-identical; that the binding is a current admitted release for the exact adapter
implementation and routing policy; and that the occurrence has no other unresolved authorization.

Four things are declared and unverifiable, all asserted by naming `EntryAbsorbing<MAX>`: that the
external system absorbs a repeat; that the adapter transmits the key; that absorption is retained
long enough; and that `Returned` is post-state-functional.

One thing is **no longer** on the trust boundary, and it is this design's main safety gain over the
rejected ones: **nothing asserts non-entry.** `SupersededBeforeEntry` is the only completion whose
wrongness silently duplicates, and this path neither produces nor consumes it. Safety rests on a
positive claim about the external system, made once, at the capability.

## What this trades away

Stated plainly, because it is the decision this design asks for.

Today, one entry per occurrence is **structural**: the legal ordinal is derived from the leaf and any
authorization while an unobserved one exists is rejected, so an Effect occurrence reaches the
external system at most once, ever, in any deployment, regardless of adapter correctness. That is one
of the few properties this repository still enforces rather than assumes.

After this change the bound is `MAX_ENTRIES`, and containment of the extra `MAX − 1` entries moves
out of the fold and into a declaration nobody can check. The kernel still bounds; it stops
guaranteeing uniqueness.

## Required alongside the mechanism

Two things must land with the mechanism rather than after it. Neither touches an invariant, but
neither is optional: the second is part of this design's safety argument, not an operational
convenience.

**Carry the subject through the barrier.** The fold computes the full identity of the parked
occurrence and then discards it: `PossibleEntry` is a unit variant, so the occurrence, path, attempt
id, and capability reference are thrown away, and the operator is told only that a run is blocked.
Making the frontier and drive outcome carry that subject costs about fifty lines, relaxes nothing,
and the access-audit projection already retains every field.

**Enumerate parked runs.** Every application entry point is per-`RunId`; there is no listing surface.
An operator who does not already hold the run id cannot find a parked run at all. Every recovery
design assumes a caller who knows which run to fix. This needs a backend index, which lands in the
SQL-ownership residual, and it is the cheapest thing here that a human actually needs. It is also
load-bearing for windowed absorption: discovery latency is the whole exposure. A park found in
minutes re-asserts inside any plausible retention window; a park found by accident weeks later
re-asserts outside it and duplicates. Without a listing surface, parks are found late by
construction, so the mechanism's trigger condition selects for exactly the parks whose absorption
has expired.

## Rejected alternatives

**A separate read-only probe component.** It permitted a duplicate effect: two workers could
authorize probe ordinals concurrently, observations commit without consulting the frontier, and the
leaf is selected by authorization order, so a late negative could follow a positive and re-invoke an
effect that provably entered. It also moved the non-entry assertion off the binding whose fencing
made it verifiable; a probe is not an adapter and could not own an authorization record truthfully;
and it required seven invariant relaxations across five fold functions.

**Re-invocation gated on `Refreshable`.** Imports a `RuntimeResourceAuthority` and a release lineage
into every capability wanting crash recovery, for systems that have neither. Makes non-entry
evidence the pivot of a mechanism that needs no evidence at all. Requires relaxing the
strict-descendant rule in the physical-binding verifier — the highest-risk edit in that plan — and
cannot populate its leaf on a parked *first* attempt, which has no lineage head.

**Guarded assignment.** Require the request to project a region, an expected prior content, and an
assigned content, so entry is self-evidencing by read-back. Strictly more expressive than a key —
it covers an object `PUT` where no idempotency key exists, and it survives key expiry because a
region does not forget. Not adopted because the kernel does not need it: the fold mechanics are
identical to the above, so the extra projections buy expressiveness for the *author*, not safety for
the *kernel*, at the cost of three mandatory projections on every resolvable request and a read-back
obligation on every adapter. It is the right guidance for how to *achieve* absorption and the wrong
shape for how to *declare* it.

**Authored recovery routes.** Never re-enter the parked occurrence. Let the certified program carry a
reconciliation branch that creates *new* occurrences with their own ordinal-zero authorizations, as
the EVM submission expansion already does. This is the only alternative that keeps one entry per
occurrence structural, forever. Not adopted because it relaxes settlement linkage — an occurrence
outcome acquires a second producer — and it charges a per-capability authoring tax that the
declaration amortizes. It remains the correct choice if the uniqueness trade above is judged
unacceptable.

**Positive adoption by read.** Deferred, not refuted. Split resolution along the asymmetry of the
read: a committed, durable, discriminating row can prove an effect *did* enter, and that positive is
stable in the direction that matters, where a negative never is. The mechanism: an optional
per-occurrence read whose only committable artifact is the ordinary observation on the
already-parked attempt — adopting the entered outcome — while a negative commits nothing, decides
nothing, and never authorizes re-entry. It needs no new record family and, alone among the
re-entering designs considered, keeps one-entry-per-occurrence structural, because it authorizes
nothing.

Deferred for two reasons. Sound attribution requires the read to discriminate *this occurrence's*
entry from an identical one someone else made, which forces a per-occurrence key into the request —
`EntryKeyed` rediscovered from the read side — and once that key exists, absorption already recovers
the same value: the adapter maps external post-state onto `Returned` on the absorbed repeat, which
is the read the probe would perform, executed by the certified adapter over an admitted binding
instead of by per-state code. The residual territory — attributable, non-absorbing, and
`Returned`-recoverable by read — has resisted a natural example. Second, observation admission is
leaf-blind and type-checked only, so the fold cannot distinguish disciplined adoption from a
fabricated `Returned`; a misattributed positive settles history on a row someone else wrote while
the real effect never enters — a dropped effect and a committed falsehood at once, with re-entry
rightly refused ever after.

The genuine residual is `EntryOnce`: adoption can rescue a parked occurrence with zero duplicate
risk, because it never writes. Reconsider it if a concrete `EntryOnce` capability accumulates parks
worth that fabrication channel. Any variant in which a *negative* read authorizes re-entry is the
separate probe above relocated to the state; relocation was never the flaw.
