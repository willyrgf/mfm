# Required RFC fixes: implementation handoff

Status: approved target design, ready for implementation.

This document is the implementation plan for the five remaining acceptance gaps in
`RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md`. It replaces the earlier issue inventory. The engineer
must implement one current design, delete every superseded path, and make the four logical commits
below. Do not add compatibility APIs, legacy decoders, feature-gated authority mints, or parallel
implementations.

`docs/design.md` remains authoritative for product behavior, `docs/architecture.md` owns placement,
`docs/code-quality.md` applies to every change, and `docs/build-and-verification.md` owns verification
selection. Where the old implementation plan says downstream code cannot construct journal DTOs,
this plan deliberately narrows the exclusion to semantic submission and authority promotion as
described under Fixes 4 and 5.

## Objective

Close all five gaps with fewer concepts and less public authority:

1. make `ProgramCatalog` the sole issuer of exact contract/codec/Rust-type associations;
2. replace raw configuration JSON with one typed ingress and typed write ownership;
3. delete unused absorbing Effect support and keep every Effect `EntryOnce` by construction;
4. replace split cloneable run/reducer correlation with one affine Store-selected owner; and
5. keep journal values as authority-free validated wire DTOs while deleting every API that accepts
   them as semantic append commands.

The work changes public Rust APIs and retained contracts. Breaking changes are intended. Reset
current fixtures and baselines and reject old shapes; do not migrate or reinterpret old data.

## Final decisions

| Fix | Decision | Consequence |
| --- | --- | --- |
| 1. Exact catalog/type association | Required | Qualification succeeds only for an association registered in the exact finalized catalog. |
| 2. Typed configuration | Required | External/retained bytes decode and validate once; local values canonicalize once; admission uses a real Store-issued configuration head. |
| 3. Absorbing identity | Remove the feature | No production capability uses absorbing entry, so delete `EntryAbsorbing` and all generic absorbing branches instead of implementing identity machinery. |
| 4. Active qualified ownership | Replace existing reduction ownership | Add no `ActiveQualifiedRun` vocabulary duplicate. Replace `ReducedRunState` with one non-Clone selected owner that contains its exact qualified prefix. |
| 5. Journal construction | Journal is an authority-free DTO crate | Public construction and strict deserialization remain valid. Store constructs records in normal execution, and no semantic mutation API accepts caller-built records or frames. |

## Material uncertainties

none

The configuration owner was resolved explicitly: the trusted composition layer persists typed
configuration, receives a Store-issued resolved head, and gives that opaque head to the
non-generic `Application`. `Application` carries the head into admission but never parses, validates,
or writes configuration. Portfolio uses `PortfolioConfig`; EVM receives a minimal domain-owned
`MfmConfig` even when its current value has no fields. Each entry point is paired with a head issued
by the same Store opening as its admission mutation path.

## Required end state

### Authority flow

```text
ProgramCatalogBuilder
  -> exact registered value associations
  -> finalized ProgramCatalog
  -> catalog-issued QualifiedTypedValue<T>

ValidatedConfig<C>
  -> ConfigurationWriteSession<C>
  -> PreparedConfigurationAppend<C>
  -> ResolvedConfiguration<C>@Head
  -> opaque ResolvedConfigurationHead
  -> Store admission

HistoryReader
  -> cloneable QualifiedRun evidence only

QualifiedHistoryPort
  -> non-Clone SelectedRun
  -> Store-built preparation/conclusion frame
  -> append/recovery owner
  -> next SelectedRun or callback-free QualifiedRun result
```

Only the final three arrows in the selected-run flow carry semantic append authority. `RunFrame`,
`RunRecord`, `RunAdmitted`, `StatePrepared`, and `StateConcluded` remain validated serialized data;
constructing one does not make it appendable.

### Public-surface rule

After the cutover, downstream code must be unable to:

- qualify a value whose exact contract and Rust type were not registered in that catalog;
- use a schema match or content match as a substitute for exact catalog association;
- construct a configuration write owner from raw JSON or an unbranded head;
- turn `QualifiedRun` evidence into a selected mutation owner;
- clone, deserialize, or construct the selected mutation owner;
- recover an opened Store or mutation port from `Runtime`;
- supply a predecessor, head sequence, occurrence, preparation ordinal, replacement reference,
  publication coordinate, or assigned preparation reference to semantic Store mutation;
- submit `RunFrame`, `StatePrepared`, or `StateConcluded` to a public semantic Store API; or
- enter a provider from anything except a direct-new committed preparation.

## Commit sequence

Implement these commits in order. Each commit must leave one coherent design and include all tests
and documentation for its change. Use the exact lower-case subjects unless the final diff makes a
small wording adjustment necessary.

1. `remove unused absorbing effect support`
2. `bind typed values to exact catalog registrations`
3. `make configuration ingress and writes typed`
4. `make selected runs own semantic append authority`

Do not split Fixes 4 and 5 into independent compatibility stages. Store-selected ownership is what
makes journal DTO construction harmless; landing only half of that boundary would preserve the
ambiguity this plan is removing.

## Commit 1 — `remove unused absorbing effect support`

### Purpose

All current production Effects are EVM broadcast operations declared as `EntryOnce`. No domain or
live adapter uses `EntryAbsorbing`. Removing the unused mode is smaller and safer than adding an
absorption-identity projection, persisted identity, replacement checks, and recovery proofs for a
feature with no consumer.

### Target contract

- `Read` retains its bounded replacement budget of at most three total attempts.
- `Effect` permits exactly one preparation and one possible provider entry.
- No public type, const generic, persisted boolean, or generic branch represents an absorbing
  Effect.
- A selected Effect preparation is never replaceable. An unresolved Effect parks permanently until
  its retained owner resolves or a supervisor discards it.

### Implementation work

#### Capabilities and Runtime

- In `crates/kernel/capabilities/src/single_trust.rs`:
  - make `EffectMode` a non-generic sealed Access mode;
  - delete `EffectEntryMode`, `EntryOnce`, `EntryAbsorbing`, their private seals, `ABSORBING`, and
    `MAX_TOTAL_ENTRIES`;
  - make Effect attempt validation accept exactly one total attempt; and
  - keep `ReadMode` bounded at one through three attempts.
- Remove the deleted exports from `crates/kernel/capabilities/src/lib.rs` and update crate rustdoc.
- In `crates/kernel/runtime/src/single_trust.rs`:
  - replace the generic `RuntimePreparationMode for EffectMode<E>` implementation with the sole
    non-generic Effect mapping;
  - stop carrying an absorbing flag into journal preparation; and
  - remove generic bounds that exist only for `EffectEntryMode`.
- Update EVM's capability declaration in `crates/domains/evm/src/lib.rs` to use the sole
  `EffectMode`. Keep the live adapter's EntryOnce behavior and diagnostics; only the unused type
  machinery disappears.

#### Program, Journal, and Store

- In `crates/kernel/program/src/single_trust.rs`:
  - remove `absorbing` and `total_attempt_bound` from `ExecutionMode::Effect`;
  - delete `is_absorbing_effect`;
  - treat every Effect as having an implicit bound of one wherever a shared bound is needed; and
  - retain `effect_domain` and fact-selection metadata.
- In `crates/kernel/journal/src/single_trust.rs`:
  - make `PreparationMode::Effect` carry no retry/absorption fields;
  - make `permits_replacement` true only for `Read`; and
  - return one as the Effect attempt bound without persisting a redundant field, if a common
    accessor remains useful.
- In `crates/kernel/store/src/single_trust.rs` and `crates/kernel/store/src/backend.rs`:
  - delete Effect replacement validation and absorbing pattern matches;
  - keep replacement preparation, liability transfer, and superseded-attempt handling only for
    `Read`; and
  - keep late Effect results bound to their one selected preparation.
- Update App program builders so `ExecutionMode::Effect` uses the new shape.

### Required deletions

- `EffectEntryMode`
- `EntryOnce`
- `EntryAbsorbing`
- `ABSORBING`
- `MAX_TOTAL_ENTRIES`
- `ExecutionMode::is_absorbing_effect`
- the persisted Effect `absorbing` field
- the persisted Effect attempt-bound field
- all tests and branches that model an absorbing replacement

Add the retired identifiers to `SINGLE_TRUST_BOUNDARY_CUTOVER_MANIFEST.md` when they can be scanned
without matching the RFC and implementation-plan archives. Update
`docs/preflight/capacity-envelope.md` so it lists only Read attempts and one-entry Effects.

### Tests

- Capability tests: Effect accepts exactly one attempt; Read accepts one through three.
- Program hostile-ingress tests: the new Effect shape accepts; old `absorbing` or Effect attempt
  fields reject through strict `deny_unknown_fields` decoding.
- Journal tests: Effect preparation has the one current shape and cannot be replaced.
- Store regression test: a second preparation for an unresolved Effect is `NotActionable` and does
  not alter reserved conclusion capacity.
- Runtime regression test: an unresolved EVM broadcast never mints a replacement call and provider
  entry remains one.
- Update canonical Program/journal fixtures and capacity measurements affected by the smaller
  retained shape.

### Focused verification

Run from the default Nix development shell while iterating:

```bash
nix develop -c cargo fmt --all -- --check
nix develop -c cargo test -p mfm-capabilities
nix develop -c cargo test -p mfm-program
nix develop -c cargo test -p mfm-journal
nix develop -c cargo test -p mfm-store
nix develop -c cargo test -p mfm-runtime
nix develop -c cargo check -p mfm-evm
git diff --check
```

Do not run `.#ci` for this commit merely because it is about to be committed.

## Commit 2 — `bind typed values to exact catalog registrations`

### Purpose

The current `ProgramCatalog::qualify<T>` checks only `T::schema_id()` against the supplied
`ContentRef`. That proves a shape and content digest, but not that the exact finalized catalog
authorized this nominal contract, Rust type, or codec. Runtime keeps additional `TypeId` checks in
its own registry, creating two partial authorities.

### Target contract

`ProgramCatalog` owns one private association table keyed by exact nominal `ContentRef`. Each entry
records the facts needed to reject transposition:

- exact nominal contract identity;
- exact schema/descriptor identity used by the canonical codec;
- exact Rust `TypeId`; and
- the exact catalog brand.

No second plugin/value registry is introduced. Runtime implementation and adapter registrations
must prove compatibility with this table at assembly finalization; they do not become an alternate
source of value authority.

### Implementation work

#### Program catalog

- Replace the brand-only `ProgramCatalog` internals in
  `crates/kernel/program/src/single_trust.rs` with one shared private inner containing the brand and
  association table.
- Make the catalog builder register value types before `finish`, using one canonical helper for the
  nominal contract identity. Prefer a small API equivalent to:

  ```rust
  builder.register_value::<T>() -> Result<ContentRef>
  ```

  Do not add a public registration descriptor hierarchy. The private entry can retain `TypeId` and
  exact schema/descriptor identity directly.
- Move the duplicated `nominal_contract_ref<T>`/`contract_ref<T>` calculations from Runtime and App
  behind the Program-owned helper. Every producer must use the same function.
- At `finish` and `ProgramCatalog::program`:
  - require every value-bearing Program contract to have one exact registration;
  - reject a contract associated with another Rust type or descriptor/codec identity;
  - reject missing, conflicting, or ambiguous registrations; and
  - allow repeated use of the same exact association without creating duplicate entries.
- Change `ProgramCatalog::qualify<T>` to look up the exact contract and compare all association
  evidence before canonicalizing the typed value.
- Add a retained-byte qualification path that performs strict canonical decoding once into `T`,
  validates the same association, and returns a catalog-branded `QualifiedTypedValue<T>`. Keep
  unchecked erased construction and downcasts private.

#### Runtime assembly and App authoring

- In `crates/kernel/runtime/src/single_trust.rs`:
  - require every registered State's `Input`, `Output`, and `Failure` and every Access capability's
    `Intent` and `Evidence` to match catalog associations;
  - keep State/capability implementation `TypeId` only as live implementation correlation, not as a
    second value registry;
  - fail `RuntimeAssemblyBuilder::finish` before any Runtime, session, or provider authority exists
    when an association is missing or conflicting; and
  - route cold reification through catalog qualification instead of accepting a schema-only match.
- In `crates/app/src/lib.rs`, centralize catalog construction for the EVM and Portfolio Programs.
  Register every concrete cumulative context, Match selector/payload, terminal result, failure,
  capability intent, and capability evidence type exactly once. Delete local nominal-contract hash
  helpers after all call sites use Program's owner.
- Update test builders throughout Store, Runtime, Replay, and App to register their exact fixture
  types before catalog finalization. Do not add an "accept all test values" escape hatch.

### Conflict rules

The implementation must make these outcomes explicit and deterministic:

| Registration/qualification case | Result |
| --- | --- |
| exact contract + exact descriptor + exact Rust type in the same catalog | accept |
| same schema but unregistered nominal contract | reject |
| exact contract registered for a different Rust type | reject |
| exact contract registered with conflicting descriptor/codec identity | reject |
| exact association from another catalog instance | reject |
| live State/capability implementation incompatible with catalog association | assembly finalization rejects |

### Tests

- Program unit tests for every row in the conflict table.
- Regression test with two Rust types intentionally using the same schema identity; only the
  registered type qualifies.
- Catalog-brand transposition test using two catalogs finalized from content-equal Programs.
- Retained-byte test proving one strict decode and exact association.
- Runtime assembly tests for missing, surplus, wrong-State-type, wrong-capability-type, and
  conflicting implementation registrations, with State/adapter/provider counters remaining zero.
- Existing non-Clone erase/downcast test updated to prove exact association in addition to brand.
- Compile-fail/API test proving downstream code cannot construct an association entry,
  `QualifiedTypedValue`, or unchecked erased value.

### Documentation

Update `docs/design.md`, `docs/architecture.md`, `crates/kernel/program/README.md`,
`crates/kernel/runtime/README.md`, and `docs/preflight/cumulative-context-abi.md`. State clearly
that a `ProgramRef`, schema ID, or content-equal Program is not catalog authority.

### Focused verification

```bash
nix develop -c cargo fmt --all -- --check
nix develop -c cargo test -p mfm-program
nix develop -c cargo test -p mfm-runtime
nix develop -c cargo test -p mfm-store
nix develop -c cargo test -p mfm-replay
nix develop -c cargo test -p mfm-app
git diff --check
```

## Commit 3 — `make configuration ingress and writes typed`

### Purpose

The Store currently validates a generic JSON `String`, computes a generic `mfm.configuration`
content identity, and exposes untyped `ConfigurationRevision`/`ConfigurationSnapshot` values. This
bypasses `MfmConfig::validate`, allows no exact configuration type association, and leaves App
admissions pointing at a fabricated `fixed-configuration-v1` reference.

### Target types and ownership

Reuse `mfm_values::MfmConfig` and `ValidatedConfig<C>`; do not create another validation trait.
Implement the smallest typed Store surface equivalent to:

```rust
pub struct ResolvedConfiguration<C: MfmConfig> { /* typed value, canonical bytes, head, Store */ }
pub struct ResolvedConfigurationHead { /* opaque read-only head identity, Store */ }
pub struct ConfigurationWriteSession<C: MfmConfig> { /* affine current global head */ }
pub struct PreparedConfigurationAppend<C: MfmConfig> { /* exact successor + command */ }
pub struct SuspendedConfigurationAppend<C: MfmConfig> { /* unknown acknowledgement owner */ }
```

`ResolvedConfigurationHead` is cloneable evidence, not write authority. Its persisted projection
must identify the exact configuration stream position and content identity. It cannot have a public
constructor. The generic resolved value remains available only where planning needs `C`; App stores
the erased head and remains non-generic.

The backend remains mechanical and byte-oriented. Raw backend row DTOs may remain public for
storage implementations, but only Store may decode them or mint typed semantic owners.

### Ingress rules

- Local typed source:
  - `ValidatedConfig<C>` validates the domain value;
  - canonicalize it exactly once;
  - do not serialize and decode it merely to reconstruct `C`.
- External JSON or retained row:
  - enforce the byte bound before allocation;
  - require strict canonical, float-free JSON;
  - deserialize exactly once into `C`;
  - run `C::validate` exactly once;
  - verify `C::schema_id`, retained content identity, secret policy, and stream coordinate; and
  - retain both typed value and canonical bytes in `ResolvedConfiguration<C>`.
- Every load, audit, write, found-same recovery, and direct-successor promotion applies the same
  1,024-revision, 16 MiB single-revision, and 64 MiB cumulative limits.
- No path parses into `serde_json::Value` as the semantic configuration representation.

### Configuration stream and write flow

- Keep configuration separate from the three-family run journal.
- Use the existing Store scope/epoch/tenant stream and global exact-head CAS. The schema identity in
  each revision is the current configuration type key; do not add a second generic stream engine.
- A typed load resolves the latest `C` at the captured global head and returns
  `ResolvedConfiguration<C>`.
- A non-Clone write session consumes a validated local value or external bytes and creates one
  `PreparedConfigurationAppend<C>` against its exact global head.
- Direct-new commit promotes the already-typed successor without readback.
- `Found` ingresses and verifies the returned retained row once before promotion.
- `StaleHead` and conflict promote nothing.
- Unknown acknowledgement returns `SuspendedConfigurationAppend<C>` retaining the exact owner and
  physical append identity.
- A write must not load or fold the complete stream merely to append one direct successor; retain
  the current cumulative byte/count accounting in the session/head metadata.

### Domain and application integration

- Change `PortfolioConfig` in `crates/domains/portfolio/src/lib.rs` to derive/implement `MfmConfig`
  and put its non-empty portfolio/quotes checks in `MfmConfig::validate`.
- Delete `ValidatedPortfolioConfig` and replace `decode_portfolio_config` with the shared
  `ValidatedConfig<PortfolioConfig>` ingress, retaining a small boundary helper only if a real
  external caller remains.
- Add the minimal domain-owned EVM configuration type in `crates/domains/evm/src/lib.rs`. It may be
  a strict zero-field struct while EVM has no configurable public value; its distinct type/schema
  still prevents a Portfolio head from being substituted.
- Replace `Application::for_tenant*` convenience construction with explicit trusted composition
  that receives, for each entry point, a `ResolvedConfigurationHead` issued by the same Store
  opening as that entry point's admission path.
- Keep `Application` non-generic. It selects the configured head by entry point and passes it to
  Store admission. It never parses or writes configuration.
- Change `RunAdmitted`'s persisted configuration field from a freely supplied `ContentRef` to the
  exact persisted head projection (stream sequence/type/content identity). Store must verify the
  opaque process-local head before constructing this projection.
- Delete `fixed-configuration-v1` and every helper/test that can admit a run with a fabricated,
  absent, foreign-Store, wrong-type, or stale-unresolved configuration head.
- Update CLI and REST bootstrap to persist explicit typed demo configuration before constructing
  the App. Keep this trusted startup configuration secret-free and non-interactive; do not add a
  credential-bearing public configuration endpoint.

### Store and PostgreSQL work

- Replace `ConfigurationRevision`, `ConfigurationSnapshot`, `PreparedConfigurationWrite`, and raw
  `prepare_append(String)` semantic APIs in `crates/kernel/store/src/single_trust.rs` and
  `crates/kernel/store/src/backend.rs` with the typed owners above.
- Keep `ConfigurationAppendCommand` borrowed and crate-constructed.
- Update `RawConfigurationRevision` only as required to carry the exact typed content identity and
  global-head accounting. Do not teach PostgreSQL about Rust types or semantic validation.
- Update the current PostgreSQL baseline and conformance fixtures. Reject old generic
  `mfm.configuration` rows; add no migration/legacy reader.
- Preserve append atomicity, Store identity/epoch checks, found/stale/unknown distinctions, and
  restart behavior.

### Tests

- `ValidatedConfig<C>` rejects domain-invalid local values before Store entry.
- External canonical JSON and retained rows decode and validate once; local typed values decode
  zero times. Use test-only owner counters at the actual ingress functions.
- Wrong schema/type, wrong content digest, noncanonical JSON, floats, secret markers, foreign Store
  head, and over-bound input reject.
- Exact and plus-one tests cover revision count, one revision, and cumulative bytes at every public
  ingress/write path.
- Direct-new, found-same, stale head, conflict, acknowledgement unknown, unknown-to-found, and
  restart load preserve the typed owner rules.
- Portfolio and EVM admissions contain the exact persisted head selected for their entry point.
- Regression test: no admission can use a fabricated `ContentRef` or a configuration head not
  present in the retained stream.
- Compile-fail tests: typed write session, prepared append, suspended append, resolved head, and
  resolved configuration cannot be field-constructed; affine write owners cannot be cloned or
  reused; `ResolvedConfiguration<A>` cannot be supplied where `ResolvedConfiguration<B>` is
  required.
- Memory/PostgreSQL conformance covers the same typed semantic dispositions over their raw rows.

### Documentation

Update `docs/design.md`, `docs/architecture.md`, `docs/persisted-public-surfaces.md`,
`docs/run-execution.md`, `docs/preflight/capacity-envelope.md`, Store/App/PostgreSQL READMEs, and
CLI/REST READMEs where startup composition changes.

### Focused verification

```bash
nix develop -c cargo fmt --all -- --check
nix develop -c cargo test -p mfm-values
nix develop -c cargo test -p mfm-program-derive
nix develop -c cargo test -p mfm-portfolio
nix develop -c cargo test -p mfm-evm
nix develop -c cargo test -p mfm-store
nix develop -c cargo test -p mfm-app
nix develop -c cargo check -p mfm
nix develop -c cargo check -p mfm-rest-api
git diff --check
```

PostgreSQL behavior is covered by the final `.#ci` managed database lane. Use a focused
package/database test earlier only when diagnosing storage behavior; do not run `.#test-db`
immediately before final `.#ci`.

## Commit 4 — `make selected runs own semantic append authority`

This commit closes Fixes 4 and 5 together.

### Current problem

`QualifiedRun` and `ReducedRunState` are independently cloneable. Public semantic methods accept
them alongside a caller-supplied expected sequence, append identity, `StatePrepared` or
`StateConcluded`, object closure, and other correlated fields. Runtime's preparation bridge passes
these as a wide argument list and reconstructs the semantic journal record itself. Store validates
the pairing at runtime, but the API represents authority as independently supplied evidence.

`OpenedStructuredStore` is also Clone and `Runtime::store()` returns it, allowing a Runtime holder
to recover the broad semantic Store surface instead of retaining only its intended execution port.

### Target selected owner

Replace `ReducedRunState` rather than adding `ActiveQualifiedRun` beside it:

```rust
pub struct SelectedRun {
    /* non-Clone/non-Serde Store brand,
       exact QualifiedRun,
       exact Program/catalog binding,
       latest typed-context evidence,
       one selected RunAction */
}
```

The exact fields remain private. Reuse existing reducer data and `RunAction`; do not build a second
reducer or action hierarchy.

Properties:

- `SelectedRun` is affine, non-Clone, non-Serde, and has no public constructor.
- It owns its `QualifiedRun`; no mutation API accepts a separate predecessor or reducer.
- It is branded to one opened Store and exact Program/catalog association.
- It represents exactly one head and zero or one action selected at that head.
- Borrowing its callback-free action is allowed; preparing or concluding consumes the owner.
- It can be destructured into cloneable `QualifiedRun` evidence only by giving up mutation
  authority.
- A `HistoryReader` returns `QualifiedRun` evidence and exposes no promotion API.
- Only the non-Clone `QualifiedHistoryPort` can load/reduce and mint `SelectedRun`.

### Store port cutover

- Remove `Clone` from `OpenedStructuredStore`.
- Keep `StructuredStore::open` followed by one consuming `split`; do not add methods that recreate
  mutation ports after the split.
- Keep `HistoryReader` cloneable and callback-free.
- Keep configuration and audit ports separate and non-authoritative for run mutation.
- Give Runtime only the exact `QualifiedHistoryPort`, not `OpenedStructuredStore`.
- Delete `Runtime::store()`. Expose only read-only identity/catalog metadata needed by trusted App
  composition; those getters must not return a mutation handle.
- When multiple Runtime assemblies target one persisted Store identity, open independent branded
  mutation ports against the same backend. They linearize through backend append-id lookup and
  exact-head CAS; do not share or transpose process-local selected owners.

### Selection and session cutover

- Replace public `reduce`, `reduce_qualified`, and separate Runtime `run + reduced` ownership with a
  single Store operation that returns `SelectedRun`.
- Change `RunSession` to retain `SelectedRun` plus its catalog-qualified typed latest value. Remove
  the duplicate `QualifiedRun`/`ReducedRunState` fields and pairing checks.
- Hot conclusion advancement consumes the prior selected owner and returns the next selected owner
  without a backend reload.
- Cold resume loads and qualifies the complete prefix once, reduces it, and returns one selected
  owner.
- Same-run race classification returns:
  - a fresh `SelectedRun` when execution may continue from the qualified latest head;
  - callback-free `QualifiedRun` evidence for conflict/invalid-history/public reporting; or
  - the unchanged append owner when acknowledgement/fact-frontier recovery remains pending.
- Preserve all current no-reexecution rules for Pure, Access, provider ingress, fact selection, and
  conclusion recovery.

### Store-owned journal construction

Keep `mfm-journal` independent and authority-free:

- retain public checked constructors/variants and strict `Serialize`/`Deserialize` for
  `RunAdmitted`, `StatePrepared`, `StateConcluded`, `RunRecord`, and `RunFrame`;
- document that these values prove bounded canonical wire structure only;
- continue using them in journal tests, replay/export, backend conformance, and storage decoding;
  and
- do not use visibility tricks, feature gates, a crate merge, or a compatibility wrapper to make
  constructors Store-only.

Move normal semantic construction into Store:

- admission accepts catalog-qualified typed input and a Store-issued resolved configuration head;
- preparation consumes the selected Access owner plus canonical typed intent/fact request/object
  material;
- Store derives occurrence, predecessor/head, sequence, preparation ordinal, replacement ref,
  execution mode/binding, reserved capacity, and physical append identity;
- conclusion consumes the matching selected/Pending owner plus coordinate-free typed
  outcome/evidence/fact proposals;
- Store derives the selected preparation ref, occurrence, append identity, publication coordinate,
  and complete frame; and
- State implementations continue to return only typed intent, outcome, evidence interpretation,
  and fact proposals.

Prefer existing coordinate-free types such as `ProposedStateOutcome`, `FactProposalSet`, typed
intent/evidence refs, and object closures. Add a new proposal DTO only if it replaces a wider
argument list and carries no coordinate or authority; do not reproduce the journal record algebra
under another name.

### APIs to delete

Delete every public semantic overload that accepts any of the following from its caller:

- `RunFrame` for admission or generic append;
- `StatePrepared`;
- `StateConcluded`;
- separate `QualifiedRun` and `ReducedRunState` values;
- `expected_sequence` or predecessor/head digest;
- preparation ordinal or `replaces`;
- assigned `PreparationRef`;
- conclusion append identity; or
- fact-publication coordinate.

At minimum this removes/replaces the current `OpenedStructuredStore` methods:

- `append_admission(RunFrame)`;
- `prepare_access*` overloads accepting a prepared record;
- `prepare_conclusion*` overloads accepting a conclusion record;
- public `reduce*`/`advance_reduced` pairing APIs;
- public `qualify_appended`/`qualify_admission` hot-promotion helpers that accept caller-built
  frames; and
- any private helper retained solely to support those public split-argument paths.

The mechanical backend remains unchanged in authority shape: `BackendAppendCommand` keeps its
crate-private constructor and storage implementations receive only borrowed checked getters.

### Cross-crate migration

- `crates/kernel/store/src/single_trust.rs`: embed the qualified prefix in `SelectedRun`, remove
  `ReducedRunState`, simplify brand/pairing checks, and centralize frame construction.
- `crates/kernel/store/src/backend.rs`: expose selection and owner-consuming mutation only through
  `QualifiedHistoryPort`; return selected/history outcomes according to the recovery matrix.
- `crates/kernel/runtime/src/single_trust.rs`: make `PreparedExecution::commit_opened` consume the
  Store-selected Access owner and delete its Store-owned coordinate arguments.
- `crates/kernel/runtime/src/lifecycle.rs`: migrate `RunSession`, `PendingConclusion`,
  `SuspendedRun`, admission, hot advance, cold resume, and race recovery to the selected owner.
- `crates/app/src/lib.rs`: stop constructing admission records/frames; compose read/configuration/
  mutation ports explicitly; keep trace/export pattern matching over journal DTOs.
- `crates/kernel/replay/src/lib.rs`: continue consuming callback-free `QualifiedRun` only. Replay
  must have no selected-owner or mutation-port dependency.
- `crates/kernel/store/src/backend_conformance.rs` and PostgreSQL tests may continue constructing
  journal DTO fixtures because they test the mechanical/wire boundary, not semantic authority.
- Remove direct `mfm-journal` dependencies from crates that no longer read or encode journal DTOs;
  retain dependencies for genuine replay, trace, export, storage, or conformance consumers.

### API and compile-fail tests

Extend the existing Store/Runtime Trybuild suites with focused cases proving downstream code cannot:

- construct, deserialize, or clone `SelectedRun`;
- promote `QualifiedRun` from `HistoryReader` into `SelectedRun`;
- call a selected mutation with a stale clone after the owner was consumed;
- recover `OpenedStructuredStore` or `QualifiedHistoryPort` from `Runtime`;
- call Store mutation with `RunFrame`, `StatePrepared`, or `StateConcluded`;
- supply head/sequence/ordinal/replacement/preparation/publication coordinates; or
- split one selected owner into both a live `RunSession` and a preparation/conclusion owner.

Replace the implementation-plan compile-fail wording that prohibited all journal construction with
the stronger relevant exclusion: construction is allowed as DTO data, but no supported State,
Runtime, App, reader, or replay path can submit or promote it into semantic mutation authority.

### Behavioral and recovery tests

- Admission: direct-new, found-same, found-different, stale semantic comparison, and unknown
  acknowledgement all use Store-built frames.
- Preparation: exactly one selected owner can prepare at its head; stale evidence cannot prepare;
  only direct-new mints `CommittedCall`.
- Read replacement: the returned selected owner fixes ordinal/replacement/budget; caller supplies
  none of them.
- Effect: remains nonreplaceable after Commit 1.
- Pure and Access conclusions: Store derives all coordinates and rejects mismatched typed proposal
  material before append ownership.
- Hot path: direct admission/preparation/conclusion advances without backend reload or full refold.
- Cold path: complete-prefix qualification returns one selected owner and invokes zero callbacks.
- Same semantic conclusion, superseded Read preparation, conflicting conclusion, invalid history,
  fact-frontier movement, and unknown acknowledgement preserve the existing recovery matrix with
  zero State/provider reentry.
- Independent Store openings with equal persisted identity cannot exchange selected/configuration/
  append owners.
- Journal tests continue to prove public checked DTO construction and strict hostile-byte rejection.

### Documentation and scan updates

Update:

- `docs/design.md`;
- `docs/architecture.md`;
- `docs/run-execution.md`;
- `docs/persisted-public-surfaces.md`;
- `docs/preflight/cumulative-context-abi.md`;
- `crates/kernel/journal/README.md`;
- `crates/kernel/store/README.md`;
- `crates/kernel/runtime/README.md`;
- `crates/kernel/replay/README.md`;
- `crates/app/README.md`;
- `IMPL_PLAN_RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md` type/API-exclusion wording; and
- `SINGLE_TRUST_BOUNDARY_CUTOVER_MANIFEST.md`, retaining `ReducedRunState` as a deleted identifier
  and adding any other fully retired authority paths.

The final architecture text must say:

- Journal owns strict wire syntax and validation, not semantic authority.
- `QualifiedRun` is cloneable callback-free evidence.
- `SelectedRun` is the sole affine Store-selected run mutation owner.
- Runtime receives only the exact mutation port and never returns it.
- Store alone supplies journal coordinates and converts coordinate-free proposals into frames.

### Focused verification

```bash
nix develop -c cargo fmt --all -- --check
nix develop -c cargo test -p mfm-journal
nix develop -c cargo test -p mfm-store
nix develop -c cargo test -p mfm-runtime
nix develop -c cargo test -p mfm-replay
nix develop -c cargo test -p mfm-app
nix develop -c cargo check -p mfm-storage-postgres --features test-support --all-targets
git diff --check
```

## Final acceptance matrix

The engineer must not declare the handoff complete until every row is evidenced.

| Requirement | Evidence |
| --- | --- |
| Unregistered or same-schema foreign Rust type cannot qualify | Program catalog unit/regression tests |
| Conflicting value/codec/implementation association fails before Runtime exists | Program + Runtime assembly tests with zero callback counters |
| External/retained config decodes once; local typed config decodes zero times | Typed configuration ingress counters/tests |
| Every configuration bound is enforced on load/write/audit/recovery | Memory and PostgreSQL exact/+1 tests |
| Every admission references an actual Store-issued typed configuration head | App/Store regression tests |
| No absorbing Effect API or retained field remains | negative scan + source/API tests |
| Effect cannot replace or enter twice | Store/Runtime behavioral tests |
| Cloneable history evidence cannot mint mutation authority | Trybuild + foreign/stale owner tests |
| Runtime cannot expose an opened Store/mutation port | Trybuild/API test |
| Store semantic APIs accept no journal record/frame command | Trybuild/API test and public API review |
| Store constructs all append coordinates and assigned preparation refs | preparation/conclusion tests inspecting retained frames |
| Direct-new remains the only provider-entry path | Runtime provider-entry counters/race tests |
| Recovery performs zero State/provider reentry | existing recovery matrix plus selected-owner regression tests |
| Journal DTO construction and strict hostile-byte validation still work | Journal unit and wire tests |
| No compatibility path or superseded symbol survives | cutover-manifest negative scan |

## Final verification and handoff report

Verification is scope-driven during implementation. Do not run broad gates before each commit, and
do not run `.#check`, `.#test`, or `.#test-db` immediately before `.#ci` on the same tree.

After all four commits are present on the final tree:

```bash
git diff --check BASE_COMMIT..HEAD
nix run .#ci
git status --short
git log --oneline BASE_COMMIT..HEAD
```

`nix run .#ci` runs exactly once as the final cross-crate gate and already includes formatting,
negative scan, Clippy, checks, unit/integration/doc tests, PostgreSQL, and capacity tasks. Run a
standalone negative scan or capacity task only earlier as a focused diagnostic; do not duplicate
either immediately before `.#ci` on the same final tree. Replace `BASE_COMMIT` with Commit 1's
parent.

The final engineer report must include:

- the four commit subjects and their responsibilities;
- public types/APIs removed, added, and replaced;
- product LOC before/after and net change, separating tests/docs when useful;
- every verification command and result;
- any test or risk left unverified;
- confirmation that no secret was logged or persisted; and
- confirmation that `Material uncertainties` remains `none`, or a stop report if implementation
  discovered a genuinely material architectural contradiction.
