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
and catalog-branded typed values. Before finalization, the catalog registers each exact nominal
contract, schema-descriptor identity, Rust value type, and capability's intent/evidence/mode/fact
association. Access declarations retain their complete immutable binding descriptor. A schema ID, `ProgramRef`, or
content-equal Program is not catalog authority, and retained bytes reenter through the same exact
association. The declaration algebra is exactly `State | Match`. Authoring fragments are expanded
before a Program is constructed; no runtime collection or ambient value map exists.
The canonical Program document has its own persisted-contract schema identity, distinct from any
root result contract. Every `RunAdmitted.program_ref` names one exact canonical `mfm.program`
object in the genesis closure; it is not reconstructed from the admitted value or current source.

Each operation supplies one domain-owned typed admission value `C0`. Every successful nonterminal
State consumes the complete current context and returns the complete next context. A Match selects
one exact closed-sum payload and continuing arms converge on one continuation contract. A zero-State
Program may return the exact admitted `C0`; every nonempty successful path ends in a terminal State.

## Three-family journal

Journal owns the strict canonical wire syntax and validation for the append-only run stream, not
semantic mutation authority. Its public checked DTOs contain only:

1. `RunAdmitted`, the separate singular genesis;
2. `StatePrepared`, an access-only preparation; and
3. `StateConcluded`, a Pure or Access conclusion.

Store alone converts catalog-qualified admission, intent, outcome, and evidence plus concrete fact
values into those DTOs for normal execution. It validates the exact capability association and
evidence-to-intent binding, derives every `ValueRef` and immutable object, and supplies occurrence, predecessor/head,
sequence, preparation ordinal/replacement/reference, append identity, and publication coordinates.
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

The admission closure contains exactly one `mfm.program` immutable object. Qualification verifies
its canonical bytes, content reference, and entry point before Store ingresses it under its exact
catalog and reduces the prefix. Missing, duplicate, substituted, noncanonical, or foreign-catalog
Program material invalidates selection; Reader, replay, export, audit, and cold resume therefore
use the retained document rather than re-planning current code.

Same-run conclusion races are classified after the latest qualified prefix, in precedence order:
`AlreadyConcludedSame`, `NoLongerSelected` for a superseded Access preparation, `Conflict`, and
`InvalidHistory`. Runtime resumes only from the returned qualified history and never re-evaluates
State or re-enters a provider during these branches. Permanent Store rejection retains a distinct
owner-bearing Runtime result until an explicit supervisor or process boundary discards it.

## Runtime and adapters

Store owns reduction and semantic evidence. Cloneable `QualifiedRun` is callback-free history
evidence and cannot be promoted through a reader. Non-Clone/non-Serde `SelectedRun` is the sole
affine Store-selected run mutation owner: it owns the exact qualified prefix, Program/catalog
association, latest typed-context evidence, selected action, and private Store-opening brand.
Giving up a selection into `QualifiedRun` destroys its mutation authority.

Runtime owns immutable live registration, the affine
`PreparedExecution`, direct-new-only `CommittedCall`, call correlation, typed input retention, and
the exhaustive `SuspendedRun` owner-fate coordinator around a Store-owned selected conclusion.
`SelectedRun`, resolved typed configurations, fact continuations, and append owners carry private
process-local identity for the exact Store opening that created them; matching
persisted scope, epoch, and tenant values alone cannot transpose a semantic owner between opens.
The Runtime-owned `PendingConclusion` is the affine handoff that retains only conclusion Store I/O
and the inert session continuation.

Pure implementations receive only typed input. Read and Effect implementations borrow input to
prepare canonical intent, then execute only after the exact preparation append is newly committed.
An adapter consumes the committed call, derives provider bytes from intent, authenticates the
response, and returns one capability-owned evidence value. Indeterminate results remain neutral;
generic errors never mint retry authority. Every Effect has exactly one possible provider entry.

An integrity-blocked Access result is a capability-certified, callback-free terminal route. The
declared failure contract supplies one static typed failure value; callers cannot choose a failure,
successor context, retry authority, or new fact publication on that route.

Program retains the complete immutable binding descriptor in each Access declaration. Runtime
registration and preparation derive it from that declaration, accept no replacement descriptor,
and the opened Store
has no generic public frame append; admission and qualified conclusion ownership are separate
ingress paths. Runtime receives only its exact non-Clone mutation port and never returns that port
or an opened Store. Multiple Runtime assemblies over one persisted identity use independent
branded openings and linearize only through backend append-id lookup and exact-head CAS.

Runtime has no scheduler, history API, per-run execution lock, or process-wide writer lease. Each
opening owns bounded active-session, deterministic CPU, planning, and provider-ingress permits;
retained-prefix qualification uses a bounded blocking job. Workers race through Store exact-head
compare-and-append. A dropped hot owner leaves only the durable prefix; cold replay qualifies the
complete prefix without callbacks. A Runtime still binds one exact live Program assembly: after
Store selects the retained document, Runtime admits it only when its retained Program reference
matches that assembly.

## Storage, tenants, and deployment

The App is a fixed-tenant facade constructed by trusted embedding. Public calls contain no
credential, principal, policy, or tenant override. PostgreSQL owns mechanical ordering within one
Store scope and writer epoch. The admitted durability claim is primary crash/restart only. Restore
or binding replacement rotates scope or epoch through an explicit persisted deployment-identity
cutover; old handles fail their next readiness/backend check and old nonterminal runs are
replay-only.

Configuration is a separate append-only stream under the same scope, epoch, and tenant. Store is
the only semantic ingress: local `ValidatedConfig<C>` values are canonicalized without decoding,
while external and retained canonical JSON is decoded once into the exact `MfmConfig` type and
validated before Store mints a resolved value or head. Each revision records that type's schema,
content digest, sequence, and cumulative bytes. Affine write sessions compare the exact global
head; direct commits promote their typed successor, found appends re-ingress the returned row, and
unknown acknowledgements retain the same physical owner. A run admission persists the selected
configuration sequence, schema, and content identity and is accepted only with a same-opening
resolved head.

EVM and Portfolio domains own their bounded cumulative contexts. EVM source States are expanded in
declaration order, and Portfolio passes one opaque continuation through each child collection.
There is no generic context type, ordered child-input bijection, or parallel workflow mechanism.
