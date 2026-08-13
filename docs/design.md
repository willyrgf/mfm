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
for each State occurrence. A conclusion is durable before its output is public.

## Runtime and adapters

Store owns reduction and semantic evidence. Runtime owns immutable live registration, the affine
`PreparedExecution`, direct-new-only `CommittedCall`, call correlation, typed input retention, and
the exhaustive `SuspendedRun` owner-fate coordinator around a Store-owned `PreparedConclusion`.

Pure implementations receive only typed input. Read and Effect implementations borrow input to
prepare canonical intent, then execute only after the exact preparation append is newly committed.
An adapter consumes the committed call, derives provider bytes from intent, authenticates the
response, and returns one capability-owned evidence value. Indeterminate results remain neutral;
generic errors never mint retry authority. `EntryOnce` is the default Effect discipline.

Access assembly retains the complete immutable binding descriptor at registration. The Runtime
preparation bridge does not accept a caller-supplied replacement descriptor, and the opened Store
has no generic public frame append; admission and qualified conclusion ownership are separate
ingress paths.

Runtime has no scheduler, history API, per-run execution lock, or process-wide writer lease.
Workers race through Store exact-head compare-and-append. A dropped hot owner leaves only the
durable prefix; cold replay qualifies the complete prefix without callbacks.

## Storage, tenants, and deployment

The App is a fixed-tenant facade constructed by trusted embedding. Public calls contain no
credential, principal, policy, or tenant override. PostgreSQL owns mechanical ordering within one
Store scope and writer epoch. The admitted durability claim is primary crash/restart only. Restore
or binding replacement rotates scope or epoch; old nonterminal runs are replay-only.

EVM and Portfolio domains own their bounded cumulative contexts. EVM source States are expanded in
declaration order, and Portfolio passes one opaque continuation through each child collection.
There is no generic context type, ordered child-input bijection, or parallel workflow mechanism.
