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
the sole exact nominal-contract/schema-descriptor/Rust-type and capability intent/evidence/mode
association tables, catalog-branded typed values, and declaration-owned binding descriptors.
Program data is callback-free and cannot invoke I/O. Runtime
State and capability registrations must match the finalized table before an assembly exists; their
private `TypeId` correlation is not a second value registry.

### Capabilities

Capabilities own one canonical intent, one closed evidence value, Read/Effect mode, fact mode, and
bounded retry/entry discipline. Evidence is sealed by representation and bound to the exact
intent and call.

### Journal and Store

Journal owns strict wire syntax and validation for canonical frames and the three public run-record
DTO families; it owns no semantic append authority. Store alone converts catalog-qualified typed
proposals into frames, derives their journal refs/object closures, validates exact capability and
evidence binding, and supplies every journal coordinate. Store owns the reducer,
sequential cursor, cumulative-context continuity, preparation selection, source-manifest-bounded
prior-fact selection, occurrence conclusion uniqueness, object/fact publication closure,
typed configuration ingress and bounds, fact-frontier preconditions, publication-coordinate assignment, and
exact-head append. Fact selections carry Store-authored producer provenance and a stream identity
bound to scope, writer epoch, and tenant; qualification checks the captured historical publication
and producer head rather than trusting retained source/value bytes alone. An opened
Store has one private brand and exposes separate non-Clone mutation, cloneable read, configuration,
and fixed-snapshot audit ports. Semantic run, reducer, configuration, fact, and append owners retain
that opening identity, so equal persisted identities do not permit same-type transposition between
independent opens. `QualifiedRun` is cloneable callback-free evidence. `SelectedRun` is the sole
affine Store-selected run mutation owner and can become cloneable evidence only by being consumed.
Its public mutation ingress is admission-specific; preparations and conclusions consume selections
and coordinate-free material only. Runtime receives only the exact mutation port and never returns
the port or an opened Store.

The configuration port keeps PostgreSQL and Memory byte-oriented while exposing only
`ResolvedConfiguration<C>`, erased `ResolvedConfigurationHead` evidence, affine typed write
sessions, and prepared/suspended append owners above that boundary. App is non-generic and only
selects same-opening heads supplied by trusted composition; it neither parses nor writes config.

### Runtime

Runtime owns immutable live assembly and affine execution owners. The only provider-entering path
requires a directly committed `CommittedCall`. `RunSession` retains the latest qualified typed
context and one `SelectedRun`; hot conclusion settlement consumes it and returns the next selection
without a backend reload, while cold resume performs one bounded complete-prefix selection. Each
opening also owns bounded
active-session, deterministic CPU, planning, and provider-ingress permits; these are backpressure,
not scheduling or per-run ownership. `SuspendedRun` is the exhaustive owner-fate coordinator for
admission, preparation, and conclusion acknowledgement boundaries; its conclusion variant is the
Runtime-owned affine `PendingConclusion`. A State implementation cannot
access Store, journal, replay, or arbitrary prior output through its supported callback. Access
registration names only the live State implementation and adapter callback; Program supplies the
immutable binding and capability association, so preparation has no caller-supplied substitution
path. Conclusion recovery preserves Store classifications for same-run races,
including identical semantic conclusions, superseded Access preparations, conflicts, and invalid
history; permanent Store rejection remains a distinct owner-bearing result rather than becoming a
retryable suspension.

### Adapters

Adapters own clients, protocol authentication, stable-key transmission, nonce/signing mechanics,
and raw response disposal. They return only bounded capability evidence or an unresolved result.
An integrity-blocked Access result consumes a capability-certified route whose typed failure is
contract-fixed and callback-free; it cannot mint a successor context, retry authority, or fact
publication.

### App and storage

App exposes a fixed-tenant structural facade. Storage reports mechanical dispositions and admitted
durability; it does not rerun Program semantics. PostgreSQL scope and writer epoch are immutable
append preconditions, not process locks. The database records the active deployment identity; a
trusted restore explicitly rotates it, making already-open old-identity handles fail closed while
preserving same-identity multiple opens.

Trusted App composition receives read, configuration, audit, and mutation ports explicitly. For an
entry backed by Runtime, its configuration head must belong to that Runtime's independent exact
opening; otherwise it must belong to App's mutation/configuration opening. Equal persisted identity
and catalog metadata establish composition compatibility but never transpose selected owners.

## Recovery

Read may use its declared bounded replacement budget. An unresolved Effect preparation parks
permanently until its retained owner resolves or a supervisor discards it. A late result for a
superseded Read preparation cannot settle an occurrence.
Pure and Access conclusions use one Store-owned append owner; duplicate conclusions are idempotent,
conflicting conclusions fail closed, and response loss before durable conclusion leaves neutral
prepared history.

### Fact and conclusion recovery matrix

| Boundary result | State/provider re-entry | Durable/session result |
| --- | ---: | --- |
| Same semantic conclusion under another physical append id | zero | qualified recorded history |
| Superseded Access preparation | zero | latest qualified history (`NoLongerSelected`) |
| Different same-occurrence conclusion | zero | qualified conflict |
| Invalid or permanently rejected owner | zero | invalid-history/permanent owner result |
| Independent fact publication moved | zero | same owner with a fresh physical fact append id |
| Ambiguous acknowledgement | zero | exact owner retained until `Found`/classification |
