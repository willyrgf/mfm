# MFM architecture

## Package ownership

```text
mfm-values + mfm-ids
        |
        v
mfm-capabilities
        |
        v
mfm-program -----> mfm-journal
        |                 |
        +------> mfm-store
                         |
                         v
                    mfm-runtime
                         |
                         v
                       mfm-app
```

Domain crates depend on Program and Capabilities. Live adapters depend on Runtime. Storage
implements Store's mechanical boundary. Replay depends on Program/Store and never Runtime. Binaries
consume App.

## Responsibilities

### Program

Program owns strict document ingress, pure expansion, State/Match declarations, contracts,
catalog-branded typed values, and binding descriptors. Program data is callback-free and cannot
invoke I/O.

### Capabilities

Capabilities own one canonical intent, one closed evidence value, Read/Effect mode, fact mode, and
bounded retry/entry discipline. Evidence is sealed by representation and bound to the exact
intent and call.

### Journal and Store

Journal owns strict canonical frames and the three run record families. Store owns the reducer,
sequential cursor, cumulative-context continuity, preparation selection, source-manifest-bounded
prior-fact selection, occurrence conclusion uniqueness, object/fact publication closure,
configuration bounds, fact-frontier preconditions, publication-coordinate assignment, and
exact-head append. An opened
Store has one private brand and exposes separate non-Clone mutation, cloneable read, configuration,
and fixed-snapshot audit ports. Semantic run, reducer, configuration, fact, and append owners retain
that opening identity, so equal persisted identities do not permit same-type transposition between
independent opens. Its public mutation ingress is admission-specific; conclusions can cross it only
through Store-owned, already-qualified conclusion owners. Runtime receives that mutation path only
through the exact opening.

### Runtime

Runtime owns immutable live assembly and affine execution owners. The only provider-entering path
requires a directly committed `CommittedCall`. `RunSession` retains the latest qualified typed
context and its reducer result; hot conclusion settlement advances that result over the new suffix,
while cold resume performs bounded complete-prefix qualification. Each opening also owns bounded
active-session, deterministic CPU, planning, and provider-ingress permits; these are backpressure,
not scheduling or per-run ownership. `SuspendedRun` is the exhaustive owner-fate coordinator for
admission, preparation, and conclusion acknowledgement boundaries; its conclusion variant is the
Runtime-owned affine `PendingConclusion`. A State implementation cannot
access Store, journal, replay, or arbitrary prior output through its supported callback. Access
registration requires the immutable binding descriptor; preparation has no caller-supplied binding
substitution path.

### Adapters

Adapters own clients, protocol authentication, stable-key transmission, nonce/signing mechanics,
and raw response disposal. They return only bounded capability evidence or an unresolved result.
An integrity-blocked Access result consumes a capability-certified route whose typed failure is
contract-fixed and callback-free; it cannot mint a successor context, retry authority, or fact
publication.

### App and storage

App exposes a fixed-tenant structural facade. Storage reports mechanical dispositions and admitted
durability; it does not rerun Program semantics. PostgreSQL scope and writer epoch are immutable
append preconditions, not process locks.

## Recovery

Read and proven absorbing Effects may use their declared bounded replacement budget. EntryOnce parks
an unresolved preparation. A late result for a superseded preparation cannot settle an occurrence.
Pure and Access conclusions use one Store-owned append owner; duplicate conclusions are idempotent,
conflicting conclusions fail closed, and response loss before durable conclusion leaves neutral
prepared history.
