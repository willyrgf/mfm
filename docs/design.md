# MFM design

MFM is a typed, append-only State runner with one byte-ingress trust boundary.

## Trust boundary

Every untrusted byte enters one owner that applies size limits, strict decoding, canonical
float-free representation, intrinsic value checks, and content identity. Provider responses also
pass protocol authentication, exact request/call binding, and bounded evidence decoding before
State code sees them. Raw provider bytes, diagnostics, credentials, signing material, and secrets
are discarded at that boundary.

The trusted process includes compiled MFM code, State implementations, adapters, transports,
signers, storage, the Rust toolchain, immutable assembly, and the tenant embedding. MFM does not
sandbox linked Rust code.

## Program and typed values

`mfm-program` owns one opaque `Program`, its strict serializable document, a callback-free catalog,
and catalog-branded typed values. The declaration algebra is exactly `State | Match`. Authoring
fragments are expanded before a Program is constructed; no runtime collection or ambient value map
exists.

Each operation supplies one domain-owned typed admission value `C0`. Every successful nonterminal
State consumes the complete current context and returns the complete next context. A Match selects
one exact closed-sum payload and continuing arms converge on one continuation contract. A zero-State
Program may return the exact admitted `C0`; every nonempty successful path ends in a terminal State.

## Three-family journal

The append-only run stream contains only:

1. `RunAdmitted`, the separate singular genesis;
2. `StatePrepared`, an access-only preparation; and
3. `StateConcluded`, a Pure or Access conclusion.

Each append contains one semantic record and its object/fact closure atomically. Store compares the
exact current head, rejects stale or conflicting logical keys, and selects at most one conclusion
for each State occurrence. A prior-fact request is projected from canonical intent, but Store fixes
its source-manifest-bounded frontier and response during preparation; State code cannot provide a
selection. A selection carries aligned Store-authored producer provenance: source identity,
producer Program, producer run/record, and the producer head at that record. Store binds its
frontier stream to scope, writer epoch, and tenant, and revalidates the publication row, producer
history, head, and exact fact content during qualification. A successful conclusion carries a
coordinate-free `FactProposalSet`; Store validates and publishes non-empty proposals under the
tenant fact head in the same append transaction. If that independent fact frontier moves during
the append, Store clears only the assigned publication coordinate, rotates the physical append
identity, and retries the same semantic conclusion owner. A conclusion is durable before its
output is public.

Same-run conclusion races are classified after the latest qualified prefix, in precedence order:
`AlreadyConcludedSame`, `NoLongerSelected` for a superseded Access preparation, `Conflict`, and
`InvalidHistory`. Runtime resumes only from the returned qualified history and never re-evaluates
State or re-enters a provider during these branches. Permanent Store rejection retains a distinct
owner-bearing Runtime result until an explicit supervisor or process boundary discards it.

## Runtime and adapters

Store owns reduction and semantic evidence. Runtime owns immutable live registration, the affine
`PreparedExecution`, direct-new-only `CommittedCall`, call correlation, typed input retention, and
the exhaustive `SuspendedRun` owner-fate coordinator around a Store-owned `PreparedConclusion`.
`QualifiedRun`, `ReducedRunState`, configuration snapshots, fact continuations, and append owners
carry private process-local identity for the exact Store opening that created them; matching
persisted scope, epoch, and tenant values alone cannot transpose a semantic owner between opens.
The Runtime-owned `PendingConclusion` is the affine handoff that retains only conclusion Store I/O
and the inert session continuation.

Pure implementations receive only typed input. Read and Effect implementations borrow input to
prepare canonical intent, then execute only after the exact preparation append is newly committed.
An adapter consumes the committed call, derives provider bytes from intent, authenticates the
response, and returns one capability-owned evidence value. Indeterminate results remain neutral;
generic errors never mint retry authority. `EntryOnce` is the default Effect discipline.

An integrity-blocked Access result is a capability-certified, callback-free terminal route. The
declared failure contract supplies one static typed failure value; callers cannot choose a failure,
successor context, retry authority, or new fact publication on that route.

Access assembly retains the complete immutable binding descriptor at registration. The Runtime
preparation bridge does not accept a caller-supplied replacement descriptor, and the opened Store
has no generic public frame append; admission and qualified conclusion ownership are separate
ingress paths.

Runtime has no scheduler, history API, per-run execution lock, or process-wide writer lease. Each
opening owns bounded active-session, deterministic CPU, planning, and provider-ingress permits;
retained-prefix qualification uses a bounded blocking job. Workers race through Store exact-head
compare-and-append. A dropped hot owner leaves only the durable prefix; cold replay qualifies the
complete prefix without callbacks.

## Storage, tenants, and deployment

The App is a fixed-tenant facade constructed by trusted embedding. Public calls contain no
credential, principal, policy, or tenant override. PostgreSQL owns mechanical ordering within one
Store scope and writer epoch. The admitted durability claim is primary crash/restart only. Restore
or binding replacement rotates scope or epoch through an explicit persisted deployment-identity
cutover; old handles fail their next readiness/backend check and old nonterminal runs are
replay-only.

EVM and Portfolio domains own their bounded cumulative contexts. EVM source States are expanded in
declaration order, and Portfolio passes one opaque continuation through each child collection.
There is no generic context type, ordered child-input bijection, or parallel workflow mechanism.
