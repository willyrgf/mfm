# Implementation plan: qualify, reduce, compile, and mechanically persist Runtime history

Status: approved target; the three pre-implementation proof gates are complete and recorded under
*Pre-implementation census result*

Target: replace the current monolithic fold, generated recoverability plane, attempt-shaped route
index, and backend semantic reducers with one ownership chain:

```text
untrusted canonical bytes
  -> owner-local typed qualification
  -> certified program and qualified immutable history
  -> one pure reduce_event rule
  -> semantic artifact intents, requirements, and explicit obligations
  -> assignment/transition/projection compiler
  -> sealed validated append
  -> atomic mechanical persistence
```

This is one coordinated clean-slate cutover. It deliberately changes schema identities, retained
bytes, portable exports, public DTOs, and the PostgreSQL destructive baseline. Old bytes are
rejected. There is no legacy decoder, migration reader, dual-write interval, literal-ID freeze,
fallback, or second reset.

All commits after `56c260ba4` are local and unpushed. Preserve a private backup ref, then rewrite
from `56c260ba4`. Harvest the useful behavior from the six Effect-entry commits; do not cherry-pick
their obsolete annex, parked-run, or attempt-index implementation. The superseded
`docs/recoverability-removal.md` is folded into this plan and stays absent, so this is the one
planning anchor.

[`AGENTS.md`](AGENTS.md), [`docs/code-quality.md`](docs/code-quality.md),
[`docs/build-and-verification.md`](docs/build-and-verification.md),
[`docs/design.md`](docs/design.md), and [`docs/architecture.md`](docs/architecture.md) are in force.

---

## Material uncertainties

none.

The three pre-implementation proof gates are complete. Their reviewed old-to-final disposition
tables are recorded in *Pre-implementation census result* below, taken against the cutover base
`56c260ba4`. Commit 2 translates them into current-only owner tables in
`docs/persisted-public-surfaces.md`.

No other target ownership or policy choice is open; the proof gates may only complete the closed
descriptor vocabulary, semantic-hash owner table, and their evidence. In particular, the portable
stream, public entry-point DTO, EVM wallet evidence, and certified-program root edge cases are
decided below; the fact request
becomes one direct typed value; the recoverability plane is deleted in the same schema reset; the
reducer has one `reduce_event` transition; provisional reductions become usable only after exact
recorded comparison and closed obligation discharge; projection receipts are rejected; and all
current runs/projections are semantically certified at open snapshot `S0`, after which sealed
same-release appends preserve the invariant inductively.

---

## Decision

### The three Runtime-store layers

The target has exactly three state-bearing layers. Qualification, reduction, compilation, and
obligation discharge are operations between them, not additional authorities:

| Layer | Representation | Authority/lifetime |
| --- | --- | --- |
| 1. Immutable history | Canonical append-only batches, objects, and configuration revisions | Sole durable semantic authority; prior bytes never change |
| 2. Verified semantic view | `VerifiedStructuredRun { QualifiedHistory, ReducedRunState }` at one exact prefix | Ephemeral/cacheable interpretation; always reproducible from layer 1 and invalidated by a different head/projection |
| 3. Current projection | Run/configuration heads, tenant-fact routes/heads, and current Effect-attention membership | Mutable disposable index; compiled from layer 2, atomically co-applied with layer-1 appends, and never trusted without compare-back |

There is no fourth “PostgreSQL semantic state,” in-memory status authority, projection receipt,
attempt lifecycle, or generated recovery contract. Layer 1 answers what happened. Layer 2 answers
what that immutable history means. Layer 3 answers where to look and supplies bounded current-set
access; every routed answer returns through layers 1 and 2.

### One owner per responsibility

| Stage | Owns | Must not own |
| --- | --- | --- |
| Canonical and IDs | Exact float-free JSON syntax, global allocation/depth limits, raw hashing, checked identity grammars | Rust/domain wire shape, workflow meaning, schema registry |
| Rust type owner | Exact fields, tags, literals, bounds, grammar, typed construction, strict canonical codec | Store locks, run continuation, global registry |
| Program/history qualification | Trust selection, canonical envelopes, hashes, object identity, certified program closure, typed retained contracts | Workflow continuation, artifact authoring, persistence, live invocation |
| `RunReducer` | Deterministic legality and continuation for one qualified immutable prefix | Verifier callbacks, authoring, IO, persistence, SQL |
| Obligation coordinator | Closed retained/current physical checks and prior-run completeness checks explicitly requested by one pending reduction | Run continuation, partial discharge, or candidate authoring |
| Transition/projection compiler | Exact deterministic records/objects, assigned assertions, fact-permit specifications, and transient store after-images from one pending reduction | Trust decisions, backend DML, affine permit minting, or alternate reduction |
| Append-only authority | Canonical run batches/objects and configuration revisions | Current membership or workflow status |
| Rebuildable projection | Exact run/config heads, tenant-fact routes/heads, current Effect-entry-attention membership | Interpreting records, leaves, entry modes, budgets, or outcomes |
| Backend | Locks, CAS, atomic visibility, immutable inserts, supplied after-image application, indexed scans | Any semantic match on Runtime/journal enums |

The append-only histories are the semantic authority. Current-set membership is non-monotonic and
therefore may be a mutable, disposable projection. Mutability is not the defect. The defect is
allowing memory code, PostgreSQL DML, index predicates, qualification SQL, and the Rust fold to each
derive membership independently.

The final invariant is:

```text
append-only canonical history
          |
          +-- qualify --> pure reduce --> compile current after-image
                                             |
                                             +-- atomic mechanical store application
                                                     |
                                                     +-- route --> load + qualify + reduce + compare
```

### What the “sole callback-free fold” does today

`crates/kernel/store/src/structured/fold.rs` is currently roughly five thousand lines and combines
responsibilities that the final design separates:

- batch envelope, predecessor, assigned-record hash, object, and closure validation;
- trusted program verification through `ProgramVerifier`;
- retained-contract decoding and a second program-schema reconciliation pass;
- the real control-flow/access/fact/closure reducer;
- physical-binding and supersession callbacks through `PublicPhysicalBindingVerifier`;
- framework object, fact, candidate, and append authoring;
- replay-versus-authoring behavior selected by `allow_generate`;
- placeholder semantic digests and record references used during generation;
- cursor, frontier, chronology, purpose, and projection derivation.

“Callback-free” currently means no live state callback, adapter, provider, signer, or destination IO.
It does not mean pure: the module calls sealed verifier traits and authors material. The target
`RunReducer` is the narrower, honest contract: a deterministic reducer over qualified immutable
history.

### Rejected local repairs

Do not implement any of these:

- `occurrence_id`/`is_tail` on `run_access_attempts`;
- a successor pointer between attempt rows;
- a PostgreSQL predicate that reconstructs current Effect semantics;
- immutable `RunProjectionReceipt` bytes or a receipt digest;
- forty literal annex schema IDs;
- “drop now, derive later” with a known gap;
- a generated annex emitted from Rust but read by no consumer;
- an append-only attention change stream as a replacement for current inventory.

Tail markers repair the reported pagination symptom but retain several reducers. Projection receipts
are derivative checkpoints excluded from journal/candidate hashes; recomputing them makes them
redundant, while trusting them makes them a second authority. A receipt digest is unkeyed and adds
no evidence. Finding the exact canonical batch already proves that its co-transactional projection
mutations committed. Current inventory remains a run-keyed mutable set because an append-only stream
cannot give a fresh operator bounded current membership without a lifetime replay or separate
materialization.

---

## Non-negotiable invariants

1. Run history still contains exactly five record families, and prior records are never mutated.
2. `CommittedBatch` plus its exact object closure remains the only run semantic authority.
3. Configuration revisions remain their own append-only authority, not a sixth run record.
4. Raw bytes, program trust, and external evidence are qualified before they can become reducer
   input or a sealed verified result.
5. `RunReducer` is synchronous, deterministic, and receives no trait object, callback, backend,
   registry, signer, provider, clock, or other ambient capability.
6. Admission and every later event, from both retained replay and incremental mutation, execute the
   same `reduce_event` rule. There is no separate start, prepare, authoring, replay, or validation
   reducer.
7. Candidate compilation and retained replay use the same compiler and owner constructors. The
   candidate's intent preview is requalified and reduced as an exact recorded event before it can be
   sealed; there is no generate-versus-validate reducer mode.
8. Physical-binding currentness, supersession, and prior-run fact completeness remain required as
   closed obligations outside the reducer. No partially discharged or provisional state escapes.
9. Every accepted append carries a transient exact expected/successor current-projection mutation
   and tenant-fact plan produced after successful reduction. Backends only compare and apply them.
10. Projection data never enters `CommittedBatch`, journal records, candidate/commit hashes,
    portable history, replay input, or exports.
11. An exact old retry never reapplies a historical after-image, rewinds a run/fact/configuration
    head, or installs a historical cached successor.
12. Every semantic read uses one backend snapshot, qualifies and reduces its canonical prefix, and
    compares the derived current projection before returning evidence.
13. Store opening retains an unsplit admitted target/release bundle, semantically certifies every
    run/projection in one pinned PostgreSQL snapshot `S0`, and freshly revalidates the release before
    capabilities escape. Same-release writes after `S0` are allowed only through sealed validated
    appends and preserve the invariant inductively; a mixed-release writer is forbidden. Missing
    indexed membership at `S0` is therefore detectable before discovery is exposed.
14. Effect remains illegal inside fan-out, so one run has at most one current
    `EffectEntryAttention`. Changing that requires a deliberate projection/schema redesign.
15. A mismatch fails closed. Normal open does not repair, backfill, dual-read, or fall back.
16. Shape-derived identity covers the normalized serialized shape and structural byte constraints.
    Audit provenance is excluded; non-structural semantic changes still require the existing manual
    schema-version decision.
17. The four domain hash formulas—run ID, fact content identity, fact logical identity, and fact
    query digest—remain explicit semantic rules and receive owner-local goldens.
18. Old histories, schemas, exports, DTOs, cursors, and database baselines are rejected after the
    one cutover. Git history is the archive.

---

## Pre-implementation resolution gate

This gate is satisfied: the three read-only inventories are complete and recorded in
*Pre-implementation census result* below, and *Material uncertainties* is `none`. This section
retains the required shape of each inventory so the recorded result can be re-reviewed against it.

The complete old-name-to-final-disposition evidence belongs in this planning anchor because it is a
cutover proof, not current platform documentation. Commit 2 must translate it into current-only
owner, predicate, and semantic-hash tables in `docs/persisted-public-surfaces.md`. Those final tables
must not retain deleted contract names merely as history. When commit 8 deletes this plan, Git
history retains the reviewed cutover census and the live documentation contains only the one current
contract; this is compatible with the zero-result deletion gates.

### Schema disposition table

For every annex entry and every production `SchemaId::new`/schema-seed site, record:

```text
contract/name
owning Rust type or typed preimage
all producers
all consumers
retained through ContentRef? yes/no
published schema identity? yes/no
current bounds/grammars/literals
final disposition: shape-derived / codec-only / delete
final owner and test
```

The census must include, at minimum:

- `SELECTOR_SCHEMA_SEED` in `crates/kernel/facts/src/selection.rs`;
- the prior-run source-manifest and scanner-binding seeds in
  `crates/kernel/journal/src/structured.rs`;
- `structured_content_ref` and Never/access-fault seeds in `mfm-spec`;
- `framework_schema_id` and `framework_object` in the current fold;
- configuration revision identity;
- every production `SchemaId::new` for a known Rust shape in signing, EVM, storage, spec,
  certification, journal, facts, store, replay, and app code.

Caller-supplied IDs for genuinely opaque external bytes are a separate category and may remain only
when their qualification contract is explicit. Test-only arbitrary IDs are not production schema
authority.

The read-only census already establishes the following baseline. The implementation gate must
confirm producer/consumer paths and fill in predicates; it must not restart from the annex's
arithmetic split or reclassify an item merely because generated documentation once published it.

| Final class | Annex indices and current contracts | Required action |
| --- | --- | --- |
| Retained/published owner identity | `2` component-object evidence; `10` fact-selection query; `15` planning profile; `20` complete portable run-export stream; `42` retained-value contract; `47` structured configuration revision | Derive a new identity from the exact Rust owner/codec and propagate the one reset |
| Live codec, no `SchemaId` | `0` admit request; `1` admit response; `5` drive response; `6` entry-point DTO; `7` error response; `12` journal head; `13` parked occurrence; `14` parked run; `19` individual portable frame; `37` public error; `38` public run view; `39` public fault attribution; `40` public fault subject; `45` access audit; `49` record ref; `50` replay result; `52` semantic head; `53` transition trace; `54` typed-value ref | Retain strict owner types/codecs and relational checks; delete identity fields, constants, lookup, and accessors |
| Primitive/preimage/validation badge | `3` content ref; `4` domain preimage; `8` fact-content preimage; `9` fact-logical preimage; `11` fact request digest preimage; `22`–`36` primitive registry nodes; `41` replay mode; `43` run-ID preimage; `44` schema descriptor | Delete schema identity/registry node; retain the rule at its checked ID, codec, or pure hash owner |
| Dead annex identity | `16` portable authorization decision; `17` fixation; `18` physical target; `21` run fixation; `46` assigned record; `48` history-object wrapper; `51` run-record wrapper | Delete outright; retain any corresponding live Rust semantic type under its actual owner |

Index `19` is deliberately codec-only in the target. Frames are never independently referenced;
the generated README's publication is obsolete, not a reason to preserve a second identity. The
complete stream at index `20` is the sole portable `ContentRef` and its descriptor must cover the
real NDJSON framing, typed payload variants, version, terminal placement, and bounds; the replay
owner separately validates the chain/fixation/seal relations.

Delete the unused retained/content-reference APIs on public `EntryPointContract`,
`FactSelectionRequest`, `FactSubject`, and `CanonicalFactPredicate`; production does not retain bytes
through those methods. Rename the public DTO to `PublishedEntryPoint` with the literal
`mfm.published-entry-point.v1`, retaining only typed construction/accessors/output encoding. This is
distinct from the retained internal `mfm.structured-entry-point-contract` certified component. Keep
the direct fact request as the value owner's retained state shape. Introduce one checked owner-local
`mfm-values::MediaType` so deleting primitive annex index `27` does not leave its grammar as ad hoc
strings; retained contracts and persisted encodings reuse it.

The census also found 47 production name-seeded `ContentRef` schema names (46 additional names
because configuration overlaps annex). The final disposition table must enumerate all of them:

- spec (24): `mfm.kernel.never-failure-contract`, `mfm.kernel.access-fault-contract`,
  `mfm.policy-proceed-placeholder`, `mfm.policy-expansion-recipe`,
  `mfm.capability-expansion-requirement`, `mfm.structured-path`,
  `mfm.closed-sum-contract`, `mfm.structured-capability-contract`,
  `mfm.structured-adapter-contract`, `mfm.structured-signer-contract`,
  `mfm.structured-resource-contract`, `mfm.secret-free-executable-identity`,
  `mfm.secret-free-qualification-artifact`, `mfm.secret-free-implementation`,
  `mfm.structured-fact-descriptor`, `mfm.structured-state-contract`,
  `mfm.authored-structured-program`, `mfm.expanded-structured-program`,
  `mfm.structured-lexical-slot`, `mfm.structured-expansion-policy`,
  `mfm.structured-expansion-profile`, `mfm.certified-program`,
  `mfm.lane-outcome-contract`, and `mfm.fan-out-join-contract`;
- certification (8): `mfm.qualified-structured-entry-point-policy`,
  `mfm.structured-entry-point-contract`, `mfm.structured-component-manifest`,
  `mfm.structured-secret-free-implementation-manifest`, `mfm.structured-kernel-contract`,
  `mfm.structured-certification-predicate-set`, `mfm.structured-expansion-proof`, and
  `mfm.structured-policy-coverage-proof`;
- fold-created objects (4): `mfm.structured-certified-program-root`,
  `mfm.structured-state-outcome`, `mfm.structured-operation-outcome`, and
  `mfm.structured-fact-claim`;
- journal (2): `mfm.prior-run-fact-source-manifest` and
  `mfm.prior-run-fact-scanner-binding`;
- facts (1): `mfm.prior-run-fact-selector-contract`;
- configuration (1): `mfm.structured-configuration-revision`;
- signing (1): `mfm.signing.generation-guarded-signer-descriptor`;
- EVM wallet (5): `mfm.evm.wallet-nonce-policy`, `mfm.evm.wallet-finality-policy`,
  `mfm.evm.wallet-assurance-policy`, `mfm.evm.deterministic-signing-profile`, and
  `mfm.evm.submission-expansion-policy`;
- EVM PostgreSQL (1): `mfm.evm.wallet-storage-evidence`.

All genuine retained component/history/policy/manifest objects in this list move to concrete owner
schemas. Delete generic factories. Delete the unused exported
`mfm.evm.wallet-finality-policy` constructor/identity; it has no production consumer.
`mfm.evm.wallet-storage-evidence` is not a
persisted schema at all: one name currently covers several tuple shapes and no raw bytes are
retained. Delete that false `ContentRef` and its redundant evidence fields without replacement as
specified in the identity cutover below.

Two duplicate-authority cases are mandatory entries, not optional cleanup:

- app admission bytes and configuration-store bytes currently use different schema derivations for
  the same `ConfigurationRevision`; one owner-derived revision reference replaces both;
- `CertifiedProgramRoot` bytes currently have both a domain-hashed `mfm.certified-program` reference
  and a raw stored `mfm.structured-certified-program-root` reference, and `RunAdmitted` retains both.
  Cut to the single representation specified below and delete the duplicate field/schema/path.

### Predicate-to-owner table

For every surviving encoded type, map each accepted-byte rule to exactly one source:

- canonical syntax/global allocation bound in `mfm-canonical`;
- ID grammar in the checked ID type;
- field/tag/literal/bound in the persisted shape and owner codec;
- structural cross-field invariant in the owner constructor;
- semantic run legality in `RunReducer`;
- ambient/current trust in an explicit obligation verifier.

No rule may remain solely in a corpus, Python generator, SQL predicate, string-selected registry, or
comment.

### Semantic-domain hash table

Inventory every production `domain_content_digest` call and every equivalent manual
`domain || canonical(value)` hash, including candidate assignment, EVM physical release,
eligibility, sender inventory, routing/registry issuance, provider assertion payloads, and wallet
reservation/activation/completion paths. For each record:

```text
literal domain
typed preimage and exact fields
semantic meaning and output brand
owner crate/function
all producers and verifiers
retained bytes/reference, if any
final disposition: typed owner operation / deleted / raw-byte primitive with justification
golden and one-field hostile mutations
```

Delete the public/caller-selected `domain_content_digest(domain: &str, value)` operation. Each
surviving semantic digest function fixes its domain internally and accepts its one typed preimage or
domain-owned fields; tests call that owner instead of the generic primitive. Pure unbranded raw-byte
hashing remains in canonical and accepts bytes, not a semantic domain string. Do not introduce a
generic “typed domain” enum shared across owners: that would be another central registry.

---

## Pre-implementation census result

Reviewed against the cutover base `56c260ba4`, which is the tree this rewrite builds on. The
superseded branch tip had appended two further annex entries — `mfm.structured-parked-occurrence.v1`
and `mfm.structured-parked-run.v1` — from the discarded attempt-index commit, so the enumerated “55”
is 53 here and the earlier index list shifts by two after `mfm.journal-head.v1`. The tables below key
on contract names rather than indices for that reason. The parked pair is deleted with its commit
and has no final owner.

### A. Annex schema disposition (53 entries)

`Ref` records whether any production path retains bytes under a `ContentRef` carrying that schema
identity; `Pub` records whether the identity itself is published outside the annex.

| Annex contract | Owning Rust type | Producers / consumers | Ref | Pub | Current predicates | Final disposition | Final owner and test |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `mfm.admit-run-request.v1` | `app::surface` admit DTO | `crates/app/src/surface.rs` | no | no | object/reject, string grammars, nullable | codec-only | app owner codec; `mfm-app` DTO round-trip/unknown-field tests |
| `mfm.admit-run-response.v1` | `app::surface` admit response | `crates/app/src/surface.rs`, `bin/cli/src/commands/run/mod.rs` | no | no | object/reject, literals | codec-only | app owner codec; CLI JSON golden |
| `mfm.component-object-evidence-contract.v1` | `mfm_values::component_object_evidence_contract_*` | `crates/kernel/values/src/retained.rs`, `values/tests/retained_contract.rs` | **yes** | **yes** | object/reject, one literal `version` | shape-derived | `mfm-values` persisted schema; descriptor mutation test |
| `mfm.content-ref.v1` | `mfm_ids::ContentRef` | annex graph only | no | no | string grammars for schema/digest | delete (primitive) | rule stays in `ContentRef`/`SchemaId`/`ContentDigest` grammars; `mfm-ids` identity tests |
| `mfm.domain-separated-preimage.v1` | none (envelope) | annex `semantic_digest` only | no | no | object `{domain, value}` | delete (preimage) | each owner hash fixes its own envelope; owner goldens |
| `mfm.drive-response.v1` | `app::surface` drive response | `crates/app/src/surface.rs`, CLI | no | no | object/reject, closed status set, nullable subject | codec-only | app owner codec; drive JSON golden |
| `mfm.entry-point-contract.v1` | `mfm_spec::public::EntryPointContract` | `crates/kernel/spec/src/public.rs` | no | **yes** | object/reject, string bounds | codec-only, renamed | `PublishedEntryPoint` with literal `mfm.published-entry-point.v1`; spec public tests |
| `mfm.error-response.v1` | CLI/REST error envelope | `bin/cli/src/presentation/output.rs`, `bin/rest-api/src/tests.rs` | no | no | object/reject, closed code set | codec-only | app-owned error DTO; CLI/REST byte-equality tests |
| `mfm.fact-content-identity-preimage.v1` | store-local preimage | `crates/kernel/store/src/structured/fact_scan.rs` | no | no | object/reject | delete (preimage) | `mfm-journal` `derive_fact_content_identity`; journal golden |
| `mfm.fact-logical-identity-preimage.v1` | store-local preimage | `crates/kernel/store/src/structured/fact_scan.rs` | no | no | object/reject | delete (preimage) | `mfm-journal` `derive_fact_logical_identity`; journal golden |
| `mfm.fact-selection-query.v1` | `mfm_facts::FactSelectionQuery` | `crates/kernel/facts/src/selection.rs` | **yes** | **yes** | object/reject, bounded arrays, closed ordering set, bounded unsigned limit | shape-derived | `mfm-facts` persisted schema; query descriptor/bounds tests |
| `mfm.fact-selection-request.v1` | `mfm_facts::FactSelectionRequest` | `crates/kernel/facts/src/selection.rs` | **yes** | **yes** | object/reject, eight scan bounds, non-empty query array | shape-derived, direct value | `mfm-facts` direct typed request; request descriptor/bounds tests |
| `mfm.journal-head.v1` | `mfm_journal::structured::JournalHead` | annex graph; journal type is live | no | no | object/reject, bounded unsigned sequence, digest grammar | codec-only | journal owner codec; journal head tests |
| `mfm.planning-profile.v1` | `mfm_spec::public` planning profile | `crates/kernel/spec/src/public.rs`, `crates/app/src/application.rs`, `bin/cli/src/commands/ops.rs` | **yes** | **yes** | object/reject, bounded arrays, invariant on `planning_profile_ref` | shape-derived | spec persisted schema; profile descriptor test |
| `mfm.portable-authorization-decision.v1` | none live | annex graph only | no | no | object/reject | delete outright | none; covered by replay authorization tests |
| `mfm.portable-fixation.v1` | none live | annex graph only | no | no | object/reject | delete outright | none; replay fixation tests |
| `mfm.portable-physical-target.v1` | none live | annex graph only | no | no | object/reject | delete outright | none; replay target tests |
| `mfm.portable-run-export-frame.v1` | `mfm_replay::portable` frame | `crates/kernel/replay/src/portable.rs` | no | **yes** | tagged union `batch`/`seal`, ordinal/chain invariants | delete identity; codec-only | `mfm-replay` closed typed frame inside `CanonicalJsonLines`; frame-union tests |
| `mfm.portable-run-export-stream.v1` | `mfm_replay::portable` stream | `crates/kernel/replay/src/portable.rs`, CLI `output_file.rs`, REST tests | **yes** | **yes** | NDJSON framing, bounds, terminal placement | shape-derived, v3 | `mfm-replay` `CanonicalJsonLines` persisted schema; stream framing/bounds tests |
| `mfm.portable-run-fixation.v1` | none live | annex graph only | no | no | object/reject | delete outright | none; replay fixation tests |
| `mfm.primitive-canonical_object_entry.v1` | canonical object entry | annex graph only | no | no | array `utf16_key` ordering, unique keys | delete (registry node) | canonical bounded-JSON terminal; canonical ordering tests |
| `mfm.primitive-canonical_value.v1` | `mfm_canonical` value | `app/surface.rs`, `spec/public.rs`, `facts/value.rs` | no | no | native float-free JSON, global bounds | delete (registry node) | canonical bounded-JSON terminal profiles; canonical hostile-ingress tests |
| `mfm.primitive-content_digest.v1` | `mfm_ids::ContentDigest` | annex graph only | no | no | `content:sha256-v1:[0-9a-f]{64}` | delete (registry node) | `mfm-ids` checked type; identity grammar tests |
| `mfm.primitive-entry_point_id.v1` | `mfm_ids` entry-point id | annex graph only | no | no | versioned entry-point grammar | delete (registry node) | `mfm-ids` checked type; identity grammar tests |
| `mfm.primitive-invocation_uuid_v4.v1` | `InvocationIdentity` | annex graph only | no | no | UUID v4 grammar | delete (registry node) | `mfm-ids` checked type; identity grammar tests |
| `mfm.primitive-media_type.v1` | ad hoc `String` today | annex graph only | no | no | lowercase registered media type, no parameters | delete (registry node); grammar relocated | new checked `mfm_values::MediaType`; media-type grammar tests |
| `mfm.primitive-occurrence_id.v1` | `mfm_ids::OccurrenceId` | annex graph only | no | no | `occurrence:sha256-jcs-v1:…` | delete (registry node) | `mfm-ids` checked type; identity grammar tests |
| `mfm.primitive-run_id.v1` | `mfm_ids::RunId` | annex graph only | no | no | `run:sha256-jcs-v1:…` | delete (registry node) | `mfm-ids` checked type; identity grammar tests |
| `mfm.primitive-schema_id.v1` | `mfm_ids::SchemaId` | annex graph only | no | no | `schema:<name>:<version>:sha256-jcs-v1:…` | delete (registry node) | `mfm-ids` checked type; identity grammar tests |
| `mfm.primitive-semantic_digest.v1` | `mfm_ids::SemanticDigest` | annex graph only | no | no | `sha256-jcs-v1:…` | delete (registry node) | `mfm-ids` checked type; identity grammar tests |
| `mfm.primitive-semantic_identity.v1` | `mfm_ids::SemanticTypeId` | annex graph only | no | no | versioned semantic identity | delete (registry node) | `mfm-ids` checked type; identity grammar tests |
| `mfm.primitive-stable_id.v1` | `mfm_ids::StableId` | `values/tests/retained_contract.rs` | no | no | `mfm.<domain>/<name>@<u64>` | delete (registry node) | `mfm-ids` checked type; identity grammar tests |
| `mfm.primitive-store_epoch.v1` | `StoreEpoch` | annex graph only | no | no | canonical u64 text | delete (registry node) | `mfm-ids` checked type; identity grammar tests |
| `mfm.primitive-store_scope_id.v1` | `StoreScopeId` | annex graph only | no | no | `mfm.store_scope.v1:[0-9a-f]{32}` | delete (registry node) | `mfm-ids` checked type; identity grammar tests |
| `mfm.primitive-tenant_scope_id.v1` | `TenantScopeId` | annex graph only | no | no | `mfm.tenant_scope.v1:[0-9a-f]{32}` | delete (registry node) | `mfm-ids` checked type; identity grammar tests |
| `mfm.public-error.v1` | `mfm_app::errors` public error | `crates/app/src/errors.rs` | no | no | object/reject, closed code set, redaction invariant | codec-only | app owner codec; redaction tests |
| `mfm.public-run-view.v1` | `app::surface` run view | `crates/app/src/surface.rs`, CLI | no | no | object/reject, closed status set | codec-only | app owner codec; run-view golden |
| `mfm.public-runtime-fault-attribution.v1` | app fault attribution | annex graph; app type is live | no | no | object/reject, nullable head | codec-only | app owner codec; fault-attribution tests |
| `mfm.public-runtime-fault-subject.v1` | app fault subject | annex graph; app type is live | no | no | object/reject, private-identity omission | codec-only | app owner codec; privacy tests |
| `mfm.replay-mode.v1` | `app::surface` replay mode | `crates/app/src/surface.rs` | no | no | closed value set | delete (badge) | Rust enum + owner codec; app replay-mode tests |
| `mfm.retained-value-contract.v1` | `mfm_values::RetainedValueContract` | `values/src/retained.rs`, `spec/src/structured.rs` | **yes** | **yes** | object/reject, media-type grammar, nested evidence ref | shape-derived | `mfm-values` persisted schema; retained-contract decode/mutation tests |
| `mfm.run-id-preimage.v1` | store-local preimage | `store/src/structured/adapter.rs` and four test copies | no | no | object of four identity strings | delete (preimage) | `mfm-journal` `derive_run_id`; pinned run-ID vector + forged-identity regression |
| `mfm.schema-descriptor.v1` | annex descriptor node | annex graph only | no | no | descriptor algebra | delete (registry node) | `mfm_values::SchemaIdentity`; descriptor mutation tests |
| `mfm.structured-access-audit.v1` | `mfm_replay::StructuredAccessAuditEntry` | `crates/kernel/replay/src/structured.rs` | no | **yes** | object/reject, closed kind/outcome sets | codec-only | replay owner codec; audit projection tests |
| `mfm.structured-assigned-record.v1` | `mfm_journal::AssignedRecord` | annex graph; journal type is live | no | no | object/reject, record hash grammar | delete outright | journal `AssignedRecord` under its owner codec; journal record tests |
| `mfm.structured-configuration-revision.v1` | `mfm_store` configuration revision | `store/src/structured/configuration.rs`, `app/src/production_structured.rs` | **yes** | **yes** | object/reject, revision-ref invariant | shape-derived, one owner | one owner-derived revision reference replacing both derivations; configuration identity test |
| `mfm.structured-history-object.v1` | `mfm_journal::HistoryObject` | annex graph; journal type is live | no | no | object/reject wrapper | delete outright | typed `HistoryObject::from_persisted`/`decode_persisted`; typed-object tests |
| `mfm.structured-record-ref.v1` | `mfm_journal::RecordRef` | annex graph; journal type is live | no | no | object/reject | codec-only | journal owner codec; record-ref tests |
| `mfm.structured-replay-result.v1` | `mfm_replay::StructuredReplayResult` | `replay/src/structured.rs`, REST tests, integration wire contract | no | **yes** | object/reject, summary invariant | codec-only | replay owner codec; replay wire-contract test |
| `mfm.structured-run-record.v1` | `mfm_journal::RunRecord` | annex graph; journal type is live | no | no | tagged union of five families | delete outright | journal `RunRecord` under its owner codec; five-family decode tests |
| `mfm.structured-semantic-head.v1` | `mfm_journal` semantic head | annex graph; journal type is live | no | no | object/reject | codec-only | journal owner codec; semantic-head tests |
| `mfm.structured-transition-trace.v1` | `mfm_replay::StructuredTransitionTrace` | `crates/kernel/replay/src/structured.rs` | no | **yes** | object/reject, projection-only invariant | codec-only | replay owner codec; trace projection tests |
| `mfm.structured-typed-value-ref.v1` | `mfm_journal::TypedValueRef` | annex graph; journal type is live | no | no | object/reject | codec-only | journal owner codec; typed-value-ref tests |

Six annex entries are genuine retained/published owner identities and are the only ones that gain a
shape-derived `SchemaKind::PersistedContract` identity: component-object evidence, fact-selection
query, fact-selection request, planning profile, portable export stream, and retained-value contract.
Configuration revision is the seventh retained identity and is name-seeded rather than annex-derived
today; it is listed in table A because the annex also publishes it. `mfm.portable-run-export-frame.v1`
is deliberately demoted to a codec-only closed union.

### B. Production name-seeded `ContentRef`/`SchemaId` sites (47)

All are `sha256(format!("mfm.structured-schema.v1:{name}:1"))` seeds or their local equivalents, so
none of them is shape-derived today.

| Owner file | Names | Final disposition |
| --- | --- | --- |
| `crates/kernel/spec/src/structured.rs` (24) | `mfm.kernel.never-failure-contract`, `mfm.kernel.access-fault-contract`, `mfm.policy-proceed-placeholder`, `mfm.policy-expansion-recipe`, `mfm.capability-expansion-requirement`, `mfm.structured-path`, `mfm.closed-sum-contract`, `mfm.structured-capability-contract`, `mfm.structured-adapter-contract`, `mfm.structured-signer-contract`, `mfm.structured-resource-contract`, `mfm.secret-free-executable-identity`, `mfm.secret-free-qualification-artifact`, `mfm.secret-free-implementation`, `mfm.structured-fact-descriptor`, `mfm.structured-state-contract`, `mfm.authored-structured-program`, `mfm.expanded-structured-program`, `mfm.structured-lexical-slot`, `mfm.structured-expansion-policy`, `mfm.structured-expansion-profile`, `mfm.certified-program`, `mfm.lane-outcome-contract`, `mfm.fan-out-join-contract` | concrete spec-owned persisted schemas; delete `structured_content_ref`; `mfm.certified-program` deleted with the root cutover |
| `crates/kernel/certify/src/structured.rs` (8) | `mfm.qualified-structured-entry-point-policy`, `mfm.structured-entry-point-contract`, `mfm.structured-component-manifest`, `mfm.structured-secret-free-implementation-manifest`, `mfm.structured-kernel-contract`, `mfm.structured-certification-predicate-set`, `mfm.structured-expansion-proof`, `mfm.structured-policy-coverage-proof` | concrete certification-owned persisted schemas; delete `typed_content_ref`; the entry-point contract replaces certification's local anonymous object |
| `crates/kernel/store/src/structured/fold.rs` (4) | `mfm.structured-certified-program-root`, `mfm.structured-state-outcome`, `mfm.structured-operation-outcome`, `mfm.structured-fact-claim` | move to compiler-owned persisted schemas; delete `framework_schema_id`/`framework_object`; the root name is deleted in favour of `mfm.certified-program-root` |
| `crates/kernel/journal/src/structured.rs` (2) | `mfm.prior-run-fact-source-manifest`, `mfm.prior-run-fact-scanner-binding` | journal-owned persisted schemas derived from the Rust shape |
| `crates/kernel/facts/src/selection.rs` (1) | `mfm.prior-run-fact-selector-contract` (`SELECTOR_SCHEMA_SEED`) | facts-owned persisted schema derived from the Rust shape |
| `crates/kernel/store/src/structured/configuration.rs` (1) | `mfm.structured-configuration-revision` | one owner-derived revision reference; `crates/app/src/production_structured.rs` stops deriving its own |
| `crates/signing/src/lib.rs` (1) | `mfm.signing.generation-guarded-signer-descriptor` | signing-owned persisted schema derived from the Rust shape |
| `crates/domains/evm/src/wallet.rs` (5) | `mfm.evm.wallet-nonce-policy`, `mfm.evm.wallet-finality-policy`, `mfm.evm.wallet-assurance-policy`, `mfm.evm.deterministic-signing-profile`, `mfm.evm.submission-expansion-policy` | EVM-owned persisted schemas; **delete** the exported `mfm.evm.wallet-finality-policy` constructor/identity, which has no production consumer |
| `crates/storages/evm-postgres/src/support.rs` (1) | `mfm.evm.wallet-storage-evidence` | **delete outright**; one name currently covers several tuple shapes and retains no bytes |

Two further seeds outside the enumerated 47 are mandatory entries and were confirmed:
`crates/live/evm/src/physical_release.rs` seeds `mfm.evm.physical-binding-release-history.v1` for a
genuinely retained release-history object and becomes an owner-derived persisted schema;
`crates/kernel/runtime/src/structured.rs` and `crates/kernel/store/src/structured/backend.rs` hash
`mfm.runtime.admission-fault.v1` and `mfm.structured-store.readiness-probe.v1` as opaque local
markers that retain no bytes and stay raw-byte primitives.

Duplicate-authority cases confirmed: app admission and configuration-store bytes derive the same
`ConfigurationRevision` identity twice (`crates/app/src/production_structured.rs` and
`crates/kernel/store/src/structured/configuration.rs`); and `CertifiedProgramRoot` carries both a
`mfm.certified-program.v1\0` domain digest (`crates/kernel/spec/src/structured.rs`) and a raw
`mfm.structured-certified-program-root` stored reference (`fold.rs`), with `RunAdmitted` retaining
both. Both cut to one representation.

Test-only arbitrary IDs (`mfm.test.*`, `mfm.fixture/*`, `mfm.portable-test.*`, `mfm.app.test-object`,
`mfm.evm-live.test-object`, `mfm.journal.test.*`, `mfm.fact-scan.test/*`, `mfm.store.boundary-object`,
`mfm.kernel.balance`, `mfm.structured-value`, `mfm.bitcoin.routing-generation`,
`mfm.evm.fixture.route`, `mfm.evm.integration-certificate`, `mfm.evm.test-wallet-*`,
`mfm.wallet-authority-provider.registry-issuance`) stay behind test support and may not be imported
by production. No caller-supplied identity for genuinely opaque external bytes survives: every
remaining site has a known Rust shape.

### C. Predicate-to-owner mapping

Every accepted-byte rule in the annex algebra maps to exactly one final source. The annex uses ten
shape kinds, sixteen grammar spellings (fifteen distinct rules), two array orderings, one integer
wire form, and one unknown-field policy. No additional closed grammar or bound form was found, so the
target vocabulary in *Complete normalized shape vocabulary* is sufficient and this plan needs no
amendment.

| Old annex predicate | Instances | Final owner |
| --- | --- | --- |
| `object { fields, unknown_fields: reject }` | 35 | owner struct + `SchemaShape::Struct` + serde `deny_unknown_fields` |
| field `presence: required` | 202 | `FieldPolicy::Required` |
| field `presence: optional_absent` | 2 | `FieldPolicy::OptionalAbsent` (distinct from a nullable value) |
| `nullable { value }` | 10 | `SchemaShape::Option` |
| `reference { contract }` | 142 | nested owner shape or typed `ContentRef` field |
| `literal { value }` | 16 | scalar literal value in the owner shape |
| `string { min_length, max_length, grammar }` | 22 | bounded UTF-8 string with a closed `StringGrammar` identifier |
| `string { min_length, max_length, values }` | 14 | Rust enum + `SchemaShape::Enum` with unit variants |
| `bounded_unsigned_integer { minimum, maximum, wire: json_integer }` | 23 | unsigned integer with inclusive range |
| `array { items, min_items, max_items, ordering: preserved, unique }` | 9 | bounded sequence with declared order and uniqueness |
| `array { ordering: utf16_key }` | 1 | closed bounded canonical-JSON terminal (object entries) |
| `tagged_union { discriminator, variants, wire_representation }` | 4 | `SchemaShape::Enum` with `EnumTagging::Internal`/`Adjacent` |
| `boolean` | 1 | `SchemaShape::Bool` |
| `canonical_decimal_u64` | 0 (algebra only) | dead; not carried over |
| global byte/depth/string/key/array/object limits | 39 constants | `mfm-canonical` syntax bounds plus the owner tables in *Redistribute all generated limits* |
| 31 prose `invariants` entries | 31 | owner constructors (structural cross-field), `RunReducer` (run legality), or obligation verifiers (ambient trust) |

Closed grammar set for the descriptor enum, all of which already have a checked Rust owner except
media type:

| Grammar identifier | Rule | Checked owner |
| --- | --- | --- |
| `ContentDigestSha256V1` | `content:sha256-v1:[0-9a-f]{64}` | `mfm_ids::ContentDigest` |
| `SemanticDigestSha256JcsV1` | `sha256-jcs-v1:[0-9a-f]{64}` | `mfm_ids::SemanticDigest` |
| `RunId` | `run:sha256-jcs-v1:[0-9a-f]{64}` | `mfm_ids::RunId` |
| `OccurrenceId` | `occurrence:sha256-jcs-v1:[0-9a-f]{64}` | `mfm_ids::OccurrenceId` |
| `SchemaId` | `schema:<name>:<version>:sha256-jcs-v1:[0-9a-f]{64}` | `mfm_ids::SchemaId` |
| `SemanticTypeId` | versioned semantic identity with `sha256-jcs-v1` digest | `mfm_ids::SemanticTypeId` |
| `EntryPointId` | `mfm.[a-z0-9._-]+.v[1-9][0-9]*` | `mfm_ids` entry-point identity |
| `StableId` | `mfm.<domain>/<name>@<positive-canonical-u64>` | `mfm_ids::StableId` |
| `StoreScopeId` | `mfm.store_scope.v1:[0-9a-f]{32}` | `mfm_ids::StoreScopeId` |
| `TenantScopeId` | `mfm.tenant_scope.v1:[0-9a-f]{32}` | `mfm_ids::TenantScopeId` |
| `UuidV4` | RFC 4122 version 4 | `InvocationIdentity` |
| `CanonicalU64Text` | `0\|[1-9][0-9]{0,19}` | `StoreEpoch` and canonical u64 text fields |
| `LowerPathToken` | `[a-z0-9][a-z0-9._/-]*` | structured path/role tokens |
| `UnicodeScalarText` | valid Unicode scalar string within its byte bounds | bounded string constructor |
| `MediaType` | lowercase registered media type without parameters | **new** `mfm_values::MediaType` |

Two bounded canonical-JSON terminal profiles are required, exactly as specified: the general
float-free signed/unsigned profile for framework surfaces, and an unsigned-native profile for
`FactSubject`, `CanonicalFactPredicate`, and their retained value contract. `PlainCanonicalJsonBytes`
alone is insufficient because it accepts signed integers and does not enforce all per-container
bounds.

### D. Semantic-domain hash disposition

Every production `domain_content_digest` caller, every `domain_digest` caller, and every manual
`domain || \0 || canonical` preimage was inventoried. After the cutover no production function
accepts a caller-provided hash domain.

| Literal domain | Current callers | Final disposition | Final owner |
| --- | --- | --- | --- |
| `mfm.run-id.v1` | `store/structured/adapter.rs`; private copies in `store/tests/structured_runtime.rs`, `postgres/tests/structured_history.rs`, `tests/integration/tests/run_identity_contract.rs`, `tests/integration/tests/evm_postgres_submission.rs`, `evm/src/submission_tests.rs` | typed owner operation | `mfm_journal::structured::derive_run_id`; pinned vector plus forged-identity regression |
| `mfm.fact-content-identity.v1` | `store/structured/fact_scan.rs` | typed owner operation | `mfm_journal::structured::derive_fact_content_identity(&CommittedFactRef)` |
| `mfm.fact-logical-identity.v1` | `store/structured/fact_scan.rs` | typed owner operation | `mfm_journal::structured::derive_fact_logical_identity(&RecordRef, &CommittedFactRef)` |
| `mfm.fact-query.v1` | `facts/src/selection.rs` | typed owner operation | `mfm_facts::derive_fact_query_digest(&FactSelectionRequest)` |
| `mfm.structured-candidate.v1` | `store/structured/fold.rs` ×2, `store/tests/structured_runtime_causal.rs` | typed owner operation | journal-owned candidate digest over a typed preimage; compiler and qualifier call it |
| `mfm.structured-record.v1` | `journal/src/structured.rs` `derive_record_hash` | already owner-fixed; typed preimage | `mfm-journal` |
| `mfm.structured-commit.v1` | `journal/src/structured.rs` `derive_commit_digest` | already owner-fixed; typed preimage | `mfm-journal` |
| `mfm.structured-access-attempt.v1` | `journal/src/structured.rs` `derive_access_attempt_id` | already owner-fixed; typed preimage | `mfm-journal` |
| `mfm.structured-semantic-state.v1` | `store/structured/fold.rs` | already owner-fixed; moves with the compiler | store compiler |
| `mfm.structured-placeholder-semantic-state.v1` | `store/structured/fold.rs` | **deleted** with placeholders | none |
| `mfm.structured-placeholder-record.v1` | `store/structured/fold.rs` | **deleted** with placeholders | none |
| `mfm.structured-placeholder-access-attempt.v1` | `store/structured/fold.rs` | **deleted** with placeholders | none |
| `mfm.certified-program.v1` | `spec/src/structured.rs` | **deleted** with the duplicate root reference | `RunAdmitted.certified_program_ref` over `mfm.certified-program-root` |
| `mfm.certified-program-closure.v1` | `certify/src/structured.rs` | keep; already owner-fixed | `canonical_component_closure_digest` |
| `mfm.failure-handler.v1`, `mfm.occurrence.v1`, `mfm.fragment-boundary.v1`, `mfm.semantic-call.v1`, `mfm.expansion-support-state.v1`, `mfm.effect.v1` | `spec/src/structured.rs` private `domain_digest` | keep; each caller already fixes its domain internally, and `domain_digest` stays private | `mfm-spec` |
| `mfm.portable-export.digest-placeholder`, `mfm.portable-export.frame-chain-genesis` | `replay/src/portable.rs` | **deleted** with the frame chain and fixed-point seal loop | none |
| `mfm.evm.candidate-replacement-eligibility.v3` | `evm/src/wallet_authority.rs` | typed owner operation; preimage loses `activation_evidence_ref` and gains `predecessor_candidate_operation_key` | `mfm-evm` wallet authority |
| `mfm.evm.wallet-observed-floor-provenance.v1` | `evm/src/wallet_authority.rs`, `evm-postgres/src/authority.rs` ×2 | **deleted** with `observed_floor_ref` | directly retained qualified floor and state input |
| `mfm.evm.wallet-reservation-evidence.v1` | `evm/src/wallet_authority.rs`, `evm-postgres/src/authority.rs` | **deleted** with `reservation_evidence_ref` | `EvmNonceReservationKey` plus retained request/state/result |
| `mfm.evm.wallet-candidate-activation-evidence.v1` | `evm/src/wallet_authority.rs` | **deleted** with `activation_evidence_ref` | `EvmCandidateOperationKey` on `ActiveWalletCandidate` |
| `mfm.evm.wallet-completion-evidence.v1` | `evm/src/wallet_authority.rs` | **deleted** with `completion_evidence_ref` | `EvmNonceCompletionKey` plus retained completion material |
| `mfm.evm.wallet-storage-evidence` | `evm-postgres/src/support.rs` `evidence_reference` | **deleted**; false `ContentRef` | none |
| `mfm.evm.structured-implementation-id.v1` | `evm/src/submission_registry.rs` | typed owner operation with a fixed internal domain | `mfm-evm` submission registry |
| `mfm.evm.fixture-sender-path-inventory.v1` | `evm/src/submission_registry.rs` fixture path | typed owner operation with a fixed internal domain, moved behind test support | `mfm-evm` test support |
| `mfm.wallet-authority-provider.assertion-payload.v1` | `evm-postgres/src/provider.rs` ×3 | typed owner operation over the complete `ProviderMutation` payload | `mfm-storage-evm-postgres` provider v4 |
| `mfm.wallet-authority-provider.assertion.v1`, `mfm.wallet-authority-provider.authentication.v1` | `evm-postgres/src/provider.rs` channel signing | keep; already owner-fixed | `mfm-storage-evm-postgres` |
| `mfm.signing.capability:sign`, `mfm.signing.sign.v1` | `crates/signing/src/lib.rs` | keep; already owner-fixed | `mfm-signing` |
| `mfm.runtime.admission-fault.v1`, `mfm.structured-store.readiness-probe.v1` | `runtime/src/structured.rs`, `store/structured/backend.rs` | keep as raw unbranded byte markers; they retain nothing | canonical raw hashing |

`mfm_journal::structured::domain_content_digest`, `mfm_canonical::RecoverabilityContract::semantic_digest`,
and `crates/storages/evm-postgres/src/support.rs::evidence_reference` are all deleted in commit 2.
Each surviving semantic digest fixes its domain internally, accepts one typed preimage, and receives
an owner golden plus one-field hostile mutations. `raw_content_digest` and its incremental hasher
become ordinary canonical byte primitives requiring no registry handle.

---

## Rust-owned persisted schemas and bounded codecs

### Correct the existing identity story

Do not implement the claims from the deleted removal document. `program-derive` name-hashes the
`SemanticTypeId`; it already builds `SchemaIdentity` from the generated shape, and
`SchemaIdentity::schema_id()` hashes the complete canonical identity including `SchemaShape`.
Program-derived value/config/input/output schema IDs are already shape-derived. The annex, journal
seeds, store framework seeds, and other known-shape seeds are the outliers.

Extend the existing `mfm-values` mechanism; do not create a new runtime registry or a second schema
crate. It is dependency-safe for `mfm-journal` to add `mfm-values`: values currently depends only on
canonical/IDs, and architecture already places values below journal.

Add exactly one `SchemaKind::PersistedContract` for retained owner types that are not an MFM state
value/config/input/output. Do not add a kind per crate/domain or reuse `PublicOutput` for retained
history. Codec-only public DTOs do not gain this kind because they have no `SchemaId`.

Add one generic persisted surface contract, conceptually:

```rust
pub trait PersistedSchema: Sized {
    fn schema_identity() -> Result<SchemaIdentity>;
    fn validate(&self) -> Result<()>;
}

pub trait CanonicalJsonPersistedSchema:
    PersistedSchema + Serialize + DeserializeOwned
{
}
```

Canonical-JSON owner types derive or share strict canonical encode/decode helpers over the subtrait;
the portable stream owner supplies its one bounded NDJSON codec around the general identity
contract. Extend `SchemaIdentity` with a closed `PersistedEncoding` rather than pretending the
complete stream is one JSON object:

```text
CanonicalJson { shape }
CanonicalJsonLines {
  record_shape,
  minimum_records,
  maximum_records,
  maximum_framed_record_bytes,
  maximum_stream_bytes,
}
```

`SchemaIdentity::schema_id()` hashes the complete encoding. `mfm-replay` owns the second
variant for the one portable stream. `CanonicalJsonLines` itself fixes LF delimiters and a required
final LF; those are not configurable flags. Its frame payload is a closed typed batch/seal union,
never an open `serde_json::Value`. Fixation/seal authorization correspondence remains owner semantic
validation; a change to that published meaning increments the manual semantic version in the same
identity.

Replace the identity's bare top-level `shape` with this encoding in the one reset (or make the old
field a private canonical-JSON accessor over it); do not retain both hash inputs. Program-derived
MFM values always construct `CanonicalJson { shape }`. Inline values continue embedding the nested
canonical JSON shape and derived ID transitively and reject a stream encoding. The existing
`validate_canonical_value` operates only on `CanonicalJson` and rejects other encodings; the replay
codec owns JSON-lines validation.

The exact public surface should remain minimal. Prefer default canonical helpers and small typed
free functions over a new untyped token or a second “stream schema” registry. Do not replace
`ValidatedCanonicalValue` with another erased “validated bytes” type. A decoded value stays its
concrete owner type.

Add typed history-object construction and decoding:

```text
HistoryObjectPayload: CanonicalJsonPersistedSchema { object_type() }
HistoryObject::from_persisted<T: HistoryObjectPayload>(&value)
HistoryObject::decode_persisted<T: HistoryObjectPayload>()
```

`CanonicalJsonPersistedSchema` is a defaulted/blanket marker over the one general contract, not a
second registry or identity mechanism. The history subtrait binds one checked object-type literal as
well as the schema, so a caller cannot pair typed bytes with a foreign object kind. Decode checks
both before returning `T`. Dynamic state values use a separate store-private constructor requiring
a certified `RetainedValueContract`/`SchemaIdentity` and qualified dynamic object kind; they never
accept an arbitrary schema-name or object-type string. Make the current arbitrary
`HistoryObject::new` and untyped `decode<T>` unavailable to production callers after migrating every
owner. Retain only a narrowly named raw hostile-fixture constructor under test support and any
explicitly qualified opaque-external constructor that the census proves necessary.

### Complete normalized shape vocabulary

Extend `SchemaShape` only with closed structural forms required by the census:

- bounded UTF-8 string with inclusive byte bounds and a closed grammar identifier;
- bounded bytes/base64url with decoded and encoded bounds where relevant;
- signed/unsigned integer with exact width or inclusive range;
- scalar literal values;
- bounded sequence with element shape, minimum/maximum cardinality, ordering, and uniqueness;
- bounded string-keyed map with key grammar/bounds, value shape, and entry bounds;
- required/optional/default field policy;
- existing struct, tuple, enum tagging, inline-value, and generic composition;
- one closed bounded canonical-JSON terminal with an explicit number profile.

Do not embed Rust paths, owner predicates, arbitrary regex text, or callback names in the identity.
Grammar identifiers must be a closed Rust enum shared by descriptor generation and checked
constructors. Audit owner/provenance remains in `SchemaAudit` and stays outside the hash.

The bounded canonical-JSON terminal has at least these profiles:

- general float-free signed/unsigned integer JSON for framework surfaces that deliberately admit it;
- unsigned-native JSON for `FactSubject`, `CanonicalFactPredicate`, and their exact retained value
  contract.

Both enforce the global byte/depth/string/key/array/object limits. `PlainCanonicalJsonBytes` is not
a sufficient replacement by itself: today it accepts signed integers and does not enforce all
per-container bounds. Strengthen canonical hostile ingress and add an owner-specific unsigned
canonical value for the fact domain. Preserve the current `FactScalar` restriction to boolean,
string, or unsigned integer.

Schema descriptor tests must prove independently that changing any field, tag, literal, bound,
grammar, number profile, referenced shape, or manual version changes the derived `SchemaId`, while
changing audit provenance does not.

### Eliminate false and duplicate identities

Apply these four decisions in the same schema reset. They are part of the authority cleanup, not
optional adjacent refactors.

1. **One portable stream identity.** Delete `PORTABLE_FRAME_SCHEMA_CONTRACT`, the individual frame
   `SchemaId`, its generated publication, and any frame `schema_id` field/accessor. Frames are
   internal typed values of the one `mfm-replay` `CanonicalJsonLines` codec. Make this an intentional
   portable v3 simplification: delete frame `ordinal`, `previous_frame_digest`,
   `final_frame_digest`, `frame_chain_digest`, `total_frames`, `total_bytes`, and the fixed-point
   seal-size loop. Physical line order, exact complete-stream identity, and the canonical batch
   predecessor chains already own those facts. Replace independent public
   `to_canonical_bytes`/`content_ref` work with `encode() -> EncodedPortableRunExport`, whose exact
   bytes and `ContentRef` are private fields exposed read-only so encoding happens once. Make raw
   `strict_decode` private or test-only. Public `verify_offline(bytes, expected_content_ref, trust)`
   requires the caller's expected complete-stream `ContentRef`, compares it against the bytes before
   decoding, then checks the
   closed batch/seal union, framing, bounds, terminal placement, source closure, fixation, and seal.
   Bump the media/version/schema to v3 and provide no v2 decoder. The portable closure digest remains
   the authorization identity; it is not a second byte identity.
2. **Public entry-point information is a DTO, not retained content.** Rename the app-facing
   `EntryPointContract` to `PublishedEntryPoint` and use the literal
   `mfm.published-entry-point.v1`. Retain typed construction, accessors, and canonical output only.
   Delete its `retained_contract`, `content_ref`, `from_canonical_json`, `Deserialize`,
   `mfm.recoverability` semantic identity/role, and imports. The distinct certified
   `mfm.structured-entry-point-contract` remains a genuine retained component: replace
   certification's local anonymous object with one concrete spec/certification-owned persisted
   type and owner codec.
3. **EVM storage evidence has no reference.** Delete `evidence_reference` and
   `mfm.evm.wallet-storage-evidence` without inventing a semantic digest or retaining new evidence
   bytes. Remove `reservation_evidence_ref`, `activation_evidence_ref`,
   `completion_evidence_ref`, `winning_activation_evidence_ref`, `observed_floor_ref`, and
   `original_terminal_witnesses_ref` from database rows, domain/provider DTOs, SQL, tests, and hashes.
   These duplicate exact retained typed preimages, request, state input, result, stable operation
   key, and signed provider mutation. Add the existing `EvmCandidateOperationKey` to
   `ActiveWalletCandidate`; replace `predecessor_activation_ref` on a replacement permit with
   `predecessor_candidate_operation_key`; identify a winner by ordinal plus transaction hash against
   the exact retained prefix. Revalidate all direct fields, closures, and signed `ProviderMutation`
   at qualification. Bump the wallet provider protocol from v3 to v4 and the EVM wallet PostgreSQL
   current baseline from v1 to v2, with strict rejection of old JSON. Rewrite the sole
   `crates/storages/evm-postgres/migrations/0001_wallet_authority.sql` baseline, its contract
   version/manifests/fixtures, and reset local wallet databases; do not add `0002` or a backfill.
   Add no evidence table/column and no replacement identity type.

   | Delete | Existing authoritative replacement |
   | --- | --- |
   | `reservation_evidence_ref` | `EvmNonceReservationKey` plus exact retained reservation request/state/result |
   | `activation_evidence_ref` | `EvmCandidateOperationKey` stored on `ActiveWalletCandidate` |
   | `predecessor_activation_ref` | `predecessor_candidate_operation_key` |
   | `winning_activation_evidence_ref` | exact prefix member selected by ordinal and transaction hash |
   | `completion_evidence_ref` | `EvmNonceCompletionKey` plus exact retained completion material |
   | `observed_floor_ref` | directly retained qualified floor and state input |
   | `original_terminal_witnesses_ref` | directly retained typed terminal witnesses |

   Candidate and completion reloads continue verifying provider attestations over the complete typed
   `ProviderMutation`. Reservation/retry qualification compares the request, state input, result,
   operation key, and SQL primary key directly. Tamper tests move to those real fields and provider
   proof instead of a derivative hash.
4. **One certified-program root reference.** Give `CertifiedProgramRoot` one concrete
   `mfm.certified-program-root` owner-derived canonical-JSON schema and the raw SHA-256 digest of its
   exact JCS bytes. Keep `RunAdmitted.certified_program_ref` and use it as root-object lookup,
   program identity, prior-run authorization identity, and export identity. Delete the bespoke
   `mfm.certified-program.v1\0` digest, `mfm.certified-program`/old
   `mfm.structured-certified-program-root` name seeds, and
   `RunAdmitted.certified_program_root_ref` plus the bespoke
   `CertifiedProgramRoot::content_ref`/`CertifiedProgramDocument::content_ref` path. Also delete
   `qualified_entry_point_admission_policy_ref`, `audit_refs`, `CertifiedProgramAuditRefs`, and
   `validate_admission_audit_refs`; they duplicate `root.components`. Keep
   `entry_point_operation_id` and require exact equality with
   `CertifiedProgram::expanded().operation_id`; keep
   and verify `canonical_component_closure_digest`.

For all four, mutate each formerly duplicated field/path in a hostile fixture before deleting it and
pin that the surviving owner already detects the mismatch. After deletion, add compile/shape tests
proving the obsolete field cannot be serialized or accepted.

### One direct fact-selection representation

`mfm-facts` owns one directly typed canonical `FactSelectionRequest` and typed query. Delete the
base64url-wrapped inner canonical document and its overlapping annex/manual decoding layers. The
direct value carries:

- exact admitted source-manifest reference;
- fixed producer scope and completeness mode as typed single-variant enums/literals;
- fixed selector contract reference;
- all eight checked scan bounds;
- a non-empty bounded ordered query collection;
- descriptor reference, typed exact predicate, optional content identity, ordering, checked limit,
  and tie-break per query.

Serde uses `deny_unknown_fields`; constructors enforce all collection/range/cross-field invariants;
the persisted schema describes their structural byte language. Preserve the exact logical
fact-query hash formula over the direct canonical request. The outer state-value bytes and schema
identity intentionally reset once.

Delete `crates/kernel/facts/src/codec.rs` registry selectors (`contract`, generic `encode`, generic
`strict_decode`) and the untyped `ValidatedCanonicalValue` fields in `FactSelectionQuery`,
`FactScalar`, `FactSubject`, and `CanonicalFactPredicate`. These types retain concrete checked fields
and canonicalize from those fields when identity or comparison is needed.

### Pure semantic hash owners

Use typed preimage structs and direct domain-separated JCS hashing. A preimage is not itself assigned
a validation-badge `SchemaId`; nested `ContentRef` values legitimately contain the schema identity
of the bytes they reference.

The table below pins the four cross-runtime/fact formulas central to this cutover. It is not an
allowlist for generic domain hashing: the pre-implementation semantic-domain table must also replace
every surviving candidate/EVM/provider formula with its own fixed owner operation and delete the
generic string-selected helper.

| Formula | Final owner | Required use |
| --- | --- | --- |
| `mfm.run-id.v1` | journal structured identity module | admission construction and hostile genesis qualification |
| `mfm.fact-content-identity.v1` | journal structured identity module | publication and fact scan |
| `mfm.fact-logical-identity.v1` | journal structured identity module | publication and fact scan |
| `mfm.fact-query.v1` | `mfm-facts` | direct request digest |

Journal owns both fact identities because their authoritative inputs are journal-owned
`CommittedFactRef`, `TypedValueRef`, and `RecordRef`. Producer independence is a property of the
content formula, not a reason to create a second owner. Keep `mfm-facts` journal-independent and
owning authored query/selector semantics; move no types and add no facts/journal dependency edge.
Add private named preimages and exactly these semantic operations:

```text
derive_fact_content_identity(&CommittedFactRef)
derive_fact_logical_identity(&RecordRef, &CommittedFactRef)
```

The logical function reads the fact's emission ordinal and internally derives its content identity,
so a caller cannot pair a foreign ordinal or digest. Publication and scanning call these same
functions; delete store-local preimages/helpers and annex calls.

The run formula remains:

```text
SHA-256(JCS({
  "domain": "mfm.run-id.v1",
  "value": {
    "entry_point_operation_id": ...,
    "invocation_identity": ...,
    "store_scope_id": ...,
    "tenant_scope_id": ...
  }
}))
```

Use the same shared `derive_run_id` in candidate admission and genesis qualification. Reject a
history whose envelope and `RunAdmitted.run_id` agree with each other but disagree with the derived
preimage. Delete every private test/app/domain/PostgreSQL copy.

Keep the following exact fixed-preimage algorithm vectors. The three fact vectors contain literal
fixture references that may cease to be production-valid after the schema reset; label them
synthetic algorithm vectors rather than old-schema compatibility contracts:

```text
run:sha256-jcs-v1:19920c0d4f97e979078021d58497a17a076024ea39b6e95af8d165798a8a70a0
fact content: sha256-jcs-v1:e2b4257cc5512992d63c9652c5355969495bdac2fd08c61f2ac7ab59cf3936d2
fact logical: sha256-jcs-v1:be2245276341502df0843773a2145aa5cb9d32599cc71fcb49efa4ce1731c303
fact query: sha256-jcs-v1:c9ec6b8fef2397e3c43d2cbc662b6bea24717a05cc29e3acb9b800bb7d480b3b
```

In commit 2, add a second set of owner-constructed production-valid goldens for fact content,
logical, and query identities using the new current descriptor/selector/value references. Those
values are expected to differ, especially the query digest whose old fixture embeds selector schema
digest `668cf10b...`. Pin the new values only after the typed current fixture is built; do not rewrite
the synthetic vectors to conceal a formula change. The run-ID vector remains production-valid
because its preimage contains no schema-qualified references.

Expose `raw_content_digest` and its incremental hasher as ordinary canonical byte primitives; they
must not require a registry handle.

### Redistribute all generated limits

Delete `crates/kernel/canonical/src/recoverability_limits.rs`. Move all 39 constants to their real
owners in the same cutover:

| Owner | Constants |
| --- | --- |
| canonical syntax/global bounds | `MAX_ARRAY_ITEMS`, `MAX_BASE64URL_CHARACTERS`, `MAX_CANONICAL_JSON_BYTES`, `MAX_CANONICAL_JSON_DEPTH`, `MAX_CANONICAL_OBJECT_KEY_UTF8_BYTES`, `MAX_OBJECT_ENTRIES`, `MAX_STRING_UTF8_BYTES` |
| store append/configuration | `MAX_STORED_FRAME_BYTES`, `MAX_BATCH_OBJECTS`, `MAX_BATCH_RECORDS`, `MAX_CONFIGURATION_REVISION_BYTES` |
| store export-evidence closure | `MAX_PORTABLE_SOURCE_RUNS`, `MAX_PORTABLE_FACT_ROUTES` |
| replay framing | `MAX_PORTABLE_EXPORT_BYTES`, `MAX_PORTABLE_FRAME_BYTES`, `MAX_PORTABLE_FRAMES`, `MAX_PORTABLE_BATCHES`, `MAX_PORTABLE_OBJECTS` |
| journal prior-run source contracts | `MAX_PRIOR_RUN_SOURCE_RULES`, `MAX_PRIOR_RUN_SOURCE_PROGRAMS_PER_RULE`, `MAX_PRIOR_RUN_SOURCE_DESCRIPTORS_PER_RULE`, `MAX_PRIOR_RUN_SOURCE_REFERENCES`, `MAX_PRIOR_RUN_SOURCE_MANIFEST_BYTES` |
| EVM wallet/provider owner | `MAX_COMPLETION_RECOVERY_BYTES`, `MAX_PROVIDER_DEPLOYMENT_ROUTES`, `MAX_PROVIDER_FINISH_AUTHORIZATION_BYTES`, `MAX_PROVIDER_MESSAGE_BYTES`, `MAX_PROVIDER_PROOF_BYTES` |
| facts | all eight `MAX_FACT_SCAN_*`, `MAX_FACT_SELECTION_QUERIES`, `MAX_FACT_SELECTION_LIMIT`, `MAX_FACT_EMISSIONS` |

Update every import in canonical, values, journal, facts, store, replay, PostgreSQL, EVM, EVM
PostgreSQL, app, and integration tests. There is no generated provenance comment and no central
cross-domain budget module after the move.

### One intentional schema and wire reset

Migrate every surviving retained type to shape-derived identity, every transport/replay DTO to its
owner codec, and every producer/consumer/fixture in one logical commit. Reset the destructive
PostgreSQL baseline and current portable/public goldens. Reject old schema identities and old wire
fields directly; do not reserve old versions or retain negative compatibility decoders.

The reset intentionally changes nested `ContentRef`s and therefore may change program roots,
history object references, candidate/commit hashes, portable bytes, and database fixtures. The four
domain hash formulas above remain pinned independently. Do not use equality with old annex IDs or
old portable bytes as acceptance.

### Remove the recoverability plane completely

Delete, without replacement artifacts or aliases:

- `crates/kernel/canonical/src/recoverability.rs` and its module/export wiring;
- `RecoverabilityContract`, `RecoverabilityError`, `RecoverabilityErrorCode`,
  `ValidatedCanonicalValue`, `CanonicalReferencePath`, `ReferenceTerminalKind`, and
  `SchemaReferenceEdge`;
- every method on that contract, including registry lookup, strict decode/encode, schema walking,
  semantic derivations, content references, and gated raw hashing;
- `contracts/recoverability/README.md`;
- `contracts/recoverability/generate.py`;
- `contracts/recoverability/v1/README.md`;
- `contracts/recoverability/v1/annex.json`;
- `contracts/recoverability/v1/corpus.json`;
- `contracts/recoverability/v1/portable_store_observed_read_audit.hex`;
- `crates/kernel/canonical/tests/recoverability_v1.rs`;
- `docs/recoverability-app-surface-v1.md`;
- `docs/recoverability-predicate-owners-v1.md`;
- every annex byte/hash/count/vector constant and generator token;
- the empty `contracts/` directory if those are its final tracked files.

Delete downstream registry-shaped helpers and errors instead of renaming them:

- `recoverability_contract`, generic string-selected `checked_value`, and
  `RecoverabilityContractUnavailable` in app surface code;
- `fixed_schema_id` in production app assembly;
- registry-backed `contract`, `validated`, and `schema_id(schema_contract: &str)` in spec;
- `RetainedValueContract::{from_validated, validated}`;
- recoverability-specific `From` conversions/error variants in canonical dependants;
- stored schema-ID fields/accessors on replay/app DTO wrappers when the ID was only a badge.

Do not run or rewrite the generator. Do not preserve annex JSON as a test fixture. Do not add the
deleted removal document's proposed known gap.

### Relocate evidence before deleting the corpus

Retain behavior, not the global oracle:

- merge `crates/kernel/ids/tests/recoverability_v1.rs` into
  `crates/kernel/ids/tests/identities.rs`, then delete only the obsolete filename and the now-unused
  `mfm-canonical` dev-dependency from `mfm-ids`;
- keep canonical exactness, duplicate-key, float, number spelling, depth, total/per-container bound,
  base64url, and raw-hasher tests under `mfm-canonical`;
- keep retained-contract exact decoding, media-type grammar, unknown-field rejection, and content
  identity under `mfm-values`;
- replace facts corpus lookup/hex helpers with typed fixtures and the explicit hash/boundary goldens
  above;
- convert portable omission, extra/reordered/substituted frame, stale head, false
  frontier/publication, duplicate fixation, identity collision, source cycle, work bound,
  semantic/audit suffix, and observed-Read vectors to named replay tests;
- keep online/offline export equality constructed from current Rust owners; do not retain frozen hex
  solely as compatibility evidence;
- replace app/CLI/REST corpus readers with typed DTO fixtures plus exact expected JSON, hostile
  unknown fields, old-field rejection, bounds, and redaction tests;
- rename tests that claim “recoverability-v1 commands/routes” to current surface language.

Delete `portable-replay-corpus` from `nixfied.nix` and `workspace-tests`: it redundantly runs the
complete replay library immediately before workspace Nextest. Rename and retain
`recoverability-postgres-v1` as `structured-history-postgres-qualification`; it is the complete
PostgreSQL structured-history target, not an annex corpus. Update
`docs/build-and-verification.md`, the PostgreSQL README, and every command in this plan.

---

## Certified program and immutable-history qualification

### Reuse the certification result; add no verifier abstraction

Move hostile bytes and trust selection in front of semantic interpretation:

```text
raw batch envelopes + raw retained objects + explicit trusted program source
  -> owner HistoryQualifier + AdmissionVerificationRegistry::verify_root
  -> QualifiedHistory { Arc<CertifiedProgram>, QualifiedBatch... }
  -> reduce_event
```

Reuse `mfm_certify::structured::CertifiedProgram`. Do not add `QualifiedProgram`,
`ProgramQualifier`, another verifier trait, or a store-local copy of certified program data. The
concrete certification-owned operation remains the only way to establish program trust:

```text
AdmissionVerificationRegistry::verify_root(
    entry_point_id,
    persisted_root,
    authored_program,
) -> CertifiedProgram
```

The store wraps that exact result in `Arc` for one qualification session. If repeated reducer scans
need indexes, add private immutable indexes to `CertifiedProgram` at the certification owner; do not
copy the expanded program into `mfm-store`. Cache programs only by exact
`(operation_id, certified_root_ref)`, and on a cache hit still compare the requested persisted root
and authored bytes with the certified material. A nominal entry-point identity is not a trust key.
Give `mfm-replay` a direct acyclic `mfm-certify` dependency and an explicit offline registry/trust
snapshot that calls the same concrete operation. There is no caller-implementable live/offline
qualification seam.

Delete:

- `ProgramVerifier`, `RegistryProgramVerifier`, `VerifiedProgramData`, `ProgramVerifierSeal`,
  `build_program_verifier`, their trait-object parameters, UI fixtures, and permissive test
  implementations;
- program-verifier callbacks during genesis;
- `validate_program_value_schemas` and fold-time `RetainedValueContract::strict_decode`;
- repeated `decode_component` scans where `CertifiedProgram` already owns the certified structure;
- any cache keyed only by nominal entry-point identity;
- every proposed or partially introduced `QualifiedProgram`/`ProgramQualifier` type.

### Keep qualified history and reduced state distinct

Use three private products with deliberately disjoint responsibilities:

```rust
struct QualifiedRunContext {
    program: Arc<CertifiedProgram>,
    admission: QualifiedAdmission,
}

struct QualifiedHistory {
    context: Arc<QualifiedRunContext>,
    batches: Vec<QualifiedBatch>,
    objects: QualifiedObjectIndex,
}

struct ReducedRunState {
    // Only semantic continuation: cursor, bindings, leaves, semantic/physical
    // heads, facts, terminal outcome, attention, and reducer indexes.
}

struct VerifiedStructuredRun {
    history: Arc<QualifiedHistory>,
    reduced: ReducedRunState,
}
```

`QualifiedRunContext` is privately constructed by the qualifier and inseparably ties the exact
`CertifiedProgram` to the admitted operation/root/authored bytes. `reduce_event` receives this
context, never a free program argument; a caller cannot combine one history with another certified
program.

`QualifiedHistory` owns the immutable evidence that purpose readers may need: qualified batches,
assigned records and their retained assertions, decoded typed objects, catalog/direct-source data,
and that tied context. It proves hostile-byte, canonical
envelope, hash, object identity/type, directly named reference, strict record decoding, and program
trust properties. It makes no workflow decision and may therefore contain a structurally valid but
semantically illegal sequence.

`ReducedRunState` contains no raw bytes, `QualifiedBatch`, retained assertion, object store, backend
handle, verifier, authoring state, or capability. It is the compact semantic continuation produced
by the reducer. `VerifiedStructuredRun` pairs the two only after every recorded event has reduced,
matched its exact compiled assertions/artifacts, and discharged its obligations. Runtime decisions
read `reduced`; audit/export/purpose readers use `history` rather than forcing raw evidence into the
reducer state.

The history qualifier owns:

- exact canonical syntax and owner-local typed decode;
- batch count/size/work bounds;
- run ID, sequence, predecessor, store scope/epoch, append request, candidate digest, assigned
  record hash, commit digest, and journal-head integrity;
- object `ContentRef`, canonical bytes, object type, uniqueness, actual batch object list, and
  availability of directly named references;
- strict five-family record decode and logical-key uniqueness;
- certified root/authored program correspondence and complete certified component closure;
- admission material references and the shared derived run-ID check;
- exact retained schema/contract correspondence;
- canonical dynamic value validation into private `QualifiedValue` values.

Qualification must not decide whether an event is legal, which semantic artifacts are required,
whether the actual introduced-object suffix is semantically complete, which leaf is current,
whether a budget remains, which obligations apply, or what attention/frontier results. It preserves
every retained derived claim in `RecordedAssertions`; it does not erase a claim merely because the
reducer/compiler can recompute it. Exact semantic object closure is established later by comparing
the qualified actual suffix with the compiler's output.

---

## Pure `RunReducer`

### Exactly one deterministic event rule

Normalize the two legitimate inputs to one private event family:

```rust
enum QualifiedEvent {
    Intent(QualifiedRuntimeIntent),
    Recorded {
        semantic: QualifiedRuntimeIntent,
        assertions: RecordedAssertions,
    },
}

enum ReducerState {
    Unadmitted,
    Admitted(ReducedRunState),
}

fn reduce_event(
    context: &QualifiedRunContext,
    previous: &ReducerState,
    event: &QualifiedEvent,
) -> Result<PendingSemanticStep>;
```

Admission is the only event legal from `Unadmitted`; every later atomic Runtime batch reduces from
`Admitted`. There is no separate `start`, `prepare_admission`, `prepare_authorization`,
`prepare_observation`, `prepare_state_transition`, replay reducer, or authoring reducer.
`QualifiedEvent::Intent` and `QualifiedEvent::Recorded` expose the same semantic input to this rule;
the latter additionally retains all assigned coordinates, references, heads, hashes, object-closure
claims, fact identities, and outcomes for exact comparison outside the reducer.

`PendingSemanticStep` is private, non-`Clone`, coordinate-free, and unsealable. It contains an
`UnboundSuccessor`, typed artifact/record intents, a closed obligation set,
`TenantFactRequirement`, and the semantic facts needed by the compiler. `UnboundSuccessor` contains
no assigned record reference, semantic/physical head, tenant coordinate, serialized object,
projection row, or post-commit capability. Neither type exposes a successor accessor.

Pin the consuming typestate; this is a compile-time boundary, not sequencing convention:

```text
PendingSemanticStep
  -> compiler compare_recorded(...) -> ComparedReduction
  -> obligation discharge(scope)    -> FinalizedReduction
  -> sealed ReducedRunState / VerifiedStructuredRun / ValidatedRunAppend
```

Preview compilation may borrow an intent `PendingSemanticStep` to author bytes, but it cannot
consume, discharge, bind, or seal it. `ComparedReduction` exists only after the compiler has
independently derived all run-local assignments, bound the permitted cross-run coordinate, compared
the intent/recorded unbound plans, and matched every retained assertion/object. It privately owns the
resulting bound `ReducedRunState` and exact obligation set, is non-`Clone`, and exposes neither.
Discharge consumes that whole compared value exactly once. Success produces `FinalizedReduction`,
the only type that exposes the bound successor. A preview cannot construct `ReducedRunState`; a
comparison/discharge fragment cannot expose one; and neither can construct `VerifiedStructuredRun`,
`ValidatedRunAppend`, a cache entry, or a fact-scan permit.

The reducer owns only deterministic semantics:

- admission/run/store/tenant semantic correspondence after structural qualification;
- declaration-ordered program walking, matches, fan-out, failure routing, lexical bindings, and
  cursor state;
- legal record-family ordering within one event and legal run transitions;
- access kind, request, attempt identity/ordinal, single-outstanding rules, refresh, closure,
  reassertion, entry budget, observation, settlement, and terminal closure;
- slot, contract, provenance, structural-origin, and state legality of `QualifiedValue` values;
- typed required-artifact and record intents, without constructing retained bytes;
- fact emission/claim semantics and unassigned tenant-fact requirements;
- semantic and physical chronology, current frontier/status, terminal outcome, direct source
  references, and `EffectEntryAttention`;
- exact descriptions of physical and prior-run completeness obligations.

The reducer must not import, accept, or call a verifier, trait object, callback, backend,
configuration port, registry, invoker, signer, provider, clock, database, async API, Runtime
proposal, raw `RunRecord`, `CommittedBatch`, `HistoryObject`, serializer, assignment API,
persistence API, `RecoverabilityContract`, `PhysicalBindingVerificationMode`, `allow_generate`, or
generated-object sink. It never decodes bytes or calls `validate_canonical_value`.

Make frontier derivation exhaustive over execution kind and state leaf. Delete the broad
`_ => BlockedIntegrity` arm. A combination that the certified program/reducer model says is
impossible is invalid history, not a synthetic integrity frontier that hides a missing transition
rule.

### One non-circular candidate path

Replace every proposal/preflight path with this exact coordinator sequence, including admission,
authorization, observation, transition, closure, reassertion, and settlement:

```text
sealed Runtime intent
  -> minimal owner intent qualification + stable run/append-request identity
  -> lookup (run_id, append_request_id)
       present: qualify/reduce the retained batch, compare intent, reload current -> ExistingSame
       absent: continue
  -> QualifiedEvent::Intent
  -> reduce_event(previous)                         [PendingSemanticStep preview]
  -> resolve required tenant frontier
  -> compiler borrow preview + ObservedFrontier -> deterministic assignments/bytes
  -> owner-qualify the exact candidate batch
  -> QualifiedEvent::Recorded
  -> reduce_event(the same previous state)          [PendingSemanticStep recorded]
  -> compiler consumes both steps, binds, compares every assertion/object -> ComparedReduction
  -> discharge ComparedReduction with RetainedAndCurrent -> FinalizedReduction
  -> build QualifiedHistory suffix + ValidatedRunAppend
  -> append
  -> release successor/post-commit capability only for NewlyCommitted
```

In detail:

1. Owner-local intent qualification validates the closed Runtime intent shape and turns it into
   `QualifiedEvent::Intent`; it performs no persistence or physical verification. For admission it
   also obtains the exact `CertifiedProgram` through the concrete registry before reduction; later
   events reuse the predecessor `QualifiedRunContext` and require the same admitted identity. This
   minimal step also derives/validates the stable `(run_id, append_request_id)` without consulting a
   current head or tenant frontier.
2. Before reducing against a current predecessor, mechanically look up that stable append request.
   If present, load and qualify the authoritative prefix through its retained batch, replay it with
   `RetainedOnly`, and require its normalized recorded semantic event and request identity to equal
   the requested intent. A mismatch is `AppendConflict`; an exact match reloads the actual current
   qualified run and returns `ExistingSame`. It never recompiles from a newer tenant frontier,
   performs a current physical check, seals the historical successor, mutates, or releases a
   capability. This is the one general append-only acknowledgement/idempotency resolver, not an
   observation preflight or compatibility path.
3. Only an absent request reaches `reduce_event` on the exact current predecessor. It returns a
   private `PendingSemanticStep` preview. The compiler may borrow it for artifact/record intents,
   unbound successor semantics, obligations, and tenant-fact requirements needed to author bytes.
   The preview is never compared as a bound state, discharged, finalized, cached, or accepted as
   evidence.
4. If the preview requires a fact barrier/publication, the coordinator reads the current tenant
   frontier. The reducer never receives that live coordinate.
5. The compiler independently derives every run-local next sequence, ordinal, reference, and hash
   from the predecessor plus event order. It uses
   `AssignmentBinding::ObservedFrontier(current_frontier)` only for the cross-run tenant coordinate,
   then materializes the owner-constructed objects, five-family records, envelope, and transient
   projection inputs.
6. The ordinary history qualifier re-reads those exact generated bytes as one `QualifiedBatch` and
   `QualifiedEvent::Recorded`. Candidate authoring receives no privileged decoder bypass.
7. The same `reduce_event` runs from the same predecessor and returns a second coordinate-free
   `PendingSemanticStep`. The compiler consumes both steps, requires their complete unbound semantic
   plans/obligations/requirements to be equal, independently derives the expected run assignments
   again, binds the observed tenant frontier, and compares every `RecordedAssertion`, actual object
   suffix/closure, record, hash, head, reference, fact, and outcome. Success alone creates
   `ComparedReduction` with the bound `ReducedRunState`.
8. The obligation coordinator consumes that `ComparedReduction` once under `RetainedAndCurrent`;
   success produces `FinalizedReduction`, the only product that exposes the successor and can build
   `ValidatedRunAppend` plus the extended immutable history.
9. The append transaction repeats `(run_id, append_request_id)` classification under its locks to
   close the absence race and compares a present candidate byte-for-byte. A successful append
   releases the successor and any post-commit capability only when the backend
   reports `NewlyCommitted`. `ExistingSame`, stale head, acknowledgement ambiguity, conflict,
   rejected evidence, or failed append reloads/returns authoritative qualified state and releases no
   preview product.

The first reduction is an authoring plan, not another semantic implementation. The second reduction
is the same rule over the retained form, so candidate acceptance already proves the exact path that
future replay will execute.

Retained replay is the strict subset:

```text
raw immutable prefix
  -> QualifiedHistory
  -> for each QualifiedEvent::Recorded in order:
       reduce_event -> PendingSemanticStep
       compiler independently derives run-local assignment
       compiler binds AssignmentBinding::RetainedCoordinate(qualified coordinate)
       compare every RecordedAssertion and exact object suffix -> ComparedReduction
       discharge RetainedOnly -> FinalizedReduction
       expose the next ReducedRunState
  -> VerifiedStructuredRun { history, reduced }
```

Incremental replay runs that same recorded sequence for exactly one qualified suffix. No separate
`verify_actionable_history`, fact-special fold, or candidate-only preparation remains.

`RetainedCoordinate` is not trusted as current and does not make comparison tautological. The
compiler still derives every run-local field independently and checks the retained coordinate's
kind/binding against the reducer requirement. The later store-global route/head comparison proves
that cross-run coordinate in both directions. `ObservedFrontier` and `RetainedCoordinate` are
compiler-only assignment bindings, not reducer modes or alternate semantic algorithms.

### Closed contextual obligation discharge

Physical validity/currentness and prior-run completeness depend on context outside pure history.
Keep them as closed obligation variants emitted by `reduce_event`:

```text
PhysicalAuthorization {
  access kind, semantic and implementation contracts, admitted route,
  optional stable lineage, optional minimum lineage head,
  optional previous physical binding, exact certificate
}

PhysicalSupersession {
  authorized binding, stable lineage, exact public lineage head,
  exact evidence, semantic/adapter identities
}

PriorRunFactCompleteness {
  authorization and captured frontier, admitted source manifest,
  typed request, exact retained response/evidence
}
```

The reducer decides when an obligation exists and binds every semantic field. An external checker
can accept or reject only that exact description; it cannot return alternate state, records, cursor,
facts, or projection.

Use one private, closed dispatcher scope:

```rust
enum ObligationDischargeScope {
    RetainedOnly,
    RetainedAndCurrent,
}
```

Do not pass that enum to a callback that can reinterpret it. The store-owned coordinator invokes
separate sealed operations such as `verify_retained_authorization`,
`verify_current_authorization`, `verify_retained_supersession`, and
`verify_current_supersession`. `RetainedOnly` invokes the retained operation;
`RetainedAndCurrent` must invoke retained first and current second. There is no `CurrentOnly` path,
so newly admitted physical evidence is necessarily valid for later retained replay.

The scope matrix is closed: retained physical authorization/supersession runs once in both scopes;
current physical authorization/supersession runs additionally only for `RetainedAndCurrent`;
`PriorRunFactCompleteness` runs exactly once at its captured immutable frontier in either scope and
never acquires a second “current” interpretation.

Discharge consumes the complete `ComparedReduction` and its whole pending obligation set exactly
once; `FinalizedReduction` is opaque and bound to both successful recorded comparison and that exact
set. There is no partial discharge, provisional verified state, public token, boolean,
caller-selected digest, or mode that can satisfy sealing. Portable and live prior-run fact
verification run the same selector obligation against different bounded source providers; neither
may substitute a weaker graph-only check. A well-formed captured source closure with a false
selected result is invalid.

Delete `PhysicalBindingVerificationMode`, `ObservationQualification`,
`RuntimeHistoryPort::qualify_observation`, writer/adapter `qualify_observation`,
`qualify_invoked_observation`, `PendingObservation`, the separate supersession preflight, and their
`Ready`, `ExistingSame`, and `InvalidSupersessionEvidence` branches. Unify mutation around the one
internal `commit_event(verified, QualifiedRuntimeIntent)` coordinator. A racing different
observation that has become obsolete reloads the winner and is not reported as a component-attributed
`CandidateRejected`/`proposal_store_fault(AppendCandidate)`.

### One transition compiler; no authoring in the reducer

The reducer emits typed semantic artifact intents. The compiler is the sole materializer and calls
the owner constructors for framework outcomes, facts, retained contracts, and every other history
object. It also owns deterministic record ordering/assignment, exact assertion comparison, and
projection/fact command compilation. The reducer never constructs or serializes a `HistoryObject`.

There is one compiler algorithm, not a generator plus validator. For a candidate it receives the
deterministically derived next coordinates; for replay it first requires the recorded coordinates
to equal those same derived coordinates. It then materializes identical expected records/objects in
both paths. Candidate bytes are emitted from that result; retained bytes are compared with it.

Delete:

- `allow_generate` and every branch conditioned on it;
- `generated_objects` and overlay generation paths;
- `framework_schema_id` and stringly `framework_object`;
- `placeholder_semantic_digest` and `placeholder_record_ref`;
- candidate/object/fact authoring helpers from the reducer;
- proposal-to-record functions embedded in interpretation;
- optional-successor preparation paths;
- `ObservationQualification` and every observation/supersession preflight listed above;
- the monolithic `fold.rs` file after its responsibilities move to the final owners.

Do not leave `fold.rs` as a facade or re-export. Prefer the final private module set under
`crates/kernel/store/src/structured/`:

- `qualification.rs` for raw/trust/type boundaries and `QualifiedHistory`;
- `reducer.rs` for `ReducerState`, `reduce_event`, and semantic products;
- `compiler.rs` for assignment, owner artifact materialization, assertion comparison, and candidate
  bytes;
- `obligations.rs` for the closed one-shot coordinator;
- `projection.rs` for minimal transient store after-images;
- `coordinator.rs`/existing mutation modules for the one candidate/replay orchestration path;
- existing purpose/backend modules for evidence projection and mechanical persistence.

Tiny private modules may be combined, but their ownership must remain singular. Current callers
import the final owners directly.

### Tenant facts and the post-commit scan permit

Pre-assignment semantics and post-assignment authority are different products. The reducer emits
only an unassigned requirement:

```rust
enum TenantFactRequirement {
    None,
    Barrier(FactScanRequirement),
    Publish(FactPublicationRequirement),
}
```

`FactScanRequirement` contains the typed request, admitted source manifest/scanner binding,
physical-binding identity, and work bounds. `FactPublicationRequirement` contains the semantic fact
set. Neither contains an assigned authorization `RecordRef`, predecessor/successor journal head,
selected tenant frontier, route order, or backend command; those do not exist at reduction time.

After the coordinator reads the tenant frontier and the compiler assigns the candidate, the compiler
binds the requirement into `TenantFactProjectionPlan` and, for a reserved scan authorization, a
transient closed specification:

```rust
struct FactScanPermitSpec {
    consumer_run_id: RunId,
    append_request_id: AppendRequestId,
    candidate_digest: ContentDigest,
    predecessor_head: Option<JournalHead>,
    successor_head: JournalHead,
    authorization_ref: RecordRef,
    physical_binding_ref: ContentRef,
    frontier: TenantFactFrontier,
    request: FactSelectionRequest,
    source_manifest: PriorRunFactSourceManifest,
    scanner_binding: PriorRunFactScannerBinding,
    bounds: FactSelectionScanBounds,
}
```

The specification is transient, retained beside the append attempt, and never enters history or
`ReducedRunState`. Only an exact `NewlyCommitted` result lets the post-commit coordinator combine it
with the bounded source provider and mint the existing affine/`FnOnce` `FactScanPermit`.
`ExistingSame`, stale/conflicting head, acknowledgement unknown, replay, failed comparison,
discharge, or append, non-reserved authorization, and arbitrary callers mint no permit. Delete every
fact-special alternate fold/load path, but retain this explicit invocation-capability boundary.

### Equivalence and sealing contract

For every typed structured-history prefix:

```text
full qualify + recorded reduce/compare/discharge(prefix + batch)
    ==
verified prefix + qualify + recorded reduce/compare/discharge(batch)
```

The resulting `ReducedRunState`, cursor, bindings, semantic/physical heads, terminal outcome,
facts, attention, obligations, artifact/record intents, tenant-fact requirement, compiled exact
records/objects/assertions, and projection plan must match. `QualifiedHistory` owns the actual
records/objects; they are not duplicated inside reduced state.

For every candidate event family, the intent preview and requalified recorded reduction must have
identical semantic plans, and candidate compilation followed by replay comparison must reproduce
the same bytes and assertions. Compile-fail/privacy tests prove no provisional `PendingSemanticStep`,
discharge fragment, or caller-built input can construct `VerifiedStructuredRun`, a validated append,
or a fact permit.

---

## Reducer-owned Effect-entry attention

Rename `PossibleEntrySubject` to `EffectEntrySubject` in place; retain no alias:

```rust
pub struct EffectEntrySubject {
    pub occurrence_id: OccurrenceId,
    pub occurrence_path_ref: ContentRef,
    pub access_attempt_id: AccessAttemptId,
    pub capability_contract_ref: ContentRef,
}

pub struct EffectEntryAttention {
    pub subject: EffectEntrySubject,
    pub resolution: EffectEntryAttentionResolution,
}

pub enum EffectEntryAttentionResolution {
    Manual,
    CloseThenReassert,
    Reassert,
}
```

`StructuredFrontier` answers what Runtime may do. `EffectEntryAttention` is an orthogonal reducer
product answering whether an Effect has unresolved possible entry requiring operator/recovery
attention. It is inventory data, not a frontier, scheduler queue, or authority.

Derive frontier and attention exhaustively from the same current `ActionableState`/`StateLeaf`:

| Effect leaf | Continuation | Attention |
| --- | --- | --- |
| `Authorized` without declared remaining absorption | `PossibleEntry` | `Manual` |
| observed `EntryUnknown` without remaining absorption | `PossibleEntry` | `Manual` |
| `EntryClosable` | `Actions` | `CloseThenReassert` |
| Effect `Reassertable` | `Actions` | `Reassert` |
| every other valid Pure/Read/Effect leaf | existing continuation | absent |
| invalid execution-kind/leaf combination | reject history | none/fallback impossible |

Do not derive attention from `StructuredFrontier`: closable/reassertable Effects are actionable and
would be omitted. `StateLeaf::Reassertable` must carry the immediately preceding
`access_attempt_id` as well as `next_attempt_ordinal`, so `Reassert` names the exact subject. A Read
may use the same leaf identity for deterministic recovery but never produces Effect attention.

Retain these legitimate concepts and names:

- `StructuredFrontier::PossibleEntry`;
- `DriveOutcome::PossibleEntry`;
- `RunEvidenceStatus::PossibleEntry` and `possible_entry` public/replay status;
- `StateLeaf::EntryUnknown` and `ObservationOutcome::EntryUnknown`;
- `EntryOnce`, spent-budget manual blocking, and conceptual parked-attempt prose;
- EVM `PriorEffectDisposition::NoUnresolvedPossibleEntry`.

Do not apply a blanket `PossibleEntry` rename.

---

## Store-issued validated append and transient projection after-image

### Unforgeable backend ingress

Replace boolean-marked `CanonicalRunAppend { committed, store_verified }` with one externally
unconstructible `ValidatedRunAppend`. It is nameable by backend crates and exposes read-only
accessors, but only the store compiler/coordinator can construct it after qualification, reduction,
artifact comparison, and obligation discharge.

Conceptually:

```rust
pub struct ValidatedRunAppend {
    committed: CommittedBatch,
    run_projection: RunProjectionMutation,
    tenant_fact_plan: TenantFactProjectionPlan,
}

pub struct RunProjectionMutation {
    expected: Option<RunCurrentProjection>,
    successor: RunCurrentProjection,
}

pub struct RunCurrentProjection {
    pub run_id: RunId,
    pub tenant_scope_id: TenantScopeId,
    pub journal_head: JournalHead,
    pub has_effect_entry_attention: bool,
}
```

These types are transient and do not implement a persisted wire contract. The full
`EffectEntryAttention` remains in the verified reducer result; persistence needs only exact current
membership for routing. A routed load qualifies and reduces history to obtain the subject and
resolution.

Delete:

- public `CanonicalRunAppend::new` and `from_store_verified`;
- `store_verified`, `is_store_verified`, and every backend verification-bit branch;
- `CanonicalRunAppend` after the validated replacement lands;
- every proposed `RunProjectionReceipt`, receipt codec, receipt bytes/digest, projection contract
  ID, receipt size bound, receipt column, receipt snapshot field, and receipt-specific test;
- any consuming accessor that discards the projection mutation and returns only the batch.

Add a compile-fail boundary proving an ordinary downstream caller cannot construct or counterfeit a
validated append.

Apply the same discipline to configuration: replace `CanonicalConfigurationAppend` with an
externally unconstructible `ValidatedConfigurationAppend` carrying the revision plus exact
expected/successor configuration heads. Do not add a configuration receipt.

### Construction seam

`crates/kernel/store/src/structured/mutation.rs::commit_prepared` is the current seam that has the
predecessor/successor knowledge and then throws it away. Move that responsibility into the unified
`commit_event` coordinator and delete `commit_prepared` after all callers move. The final path must:

1. derive the expected run projection from the verified predecessor, or `None` for admission;
2. take the bound successor projection exposed only by `FinalizedReduction`;
3. take the exact tenant-fact plan already bound by the compiler and carried through comparison/
   discharge;
4. consume the final proof that the requalified batch/object closure and all recorded assignments
   equal compiler output;
5. construct `ValidatedRunAppend` privately;
6. dispatch it to the backend.

No backend receives a candidate without a finalized successor. `has_effect_entry_attention` is
exactly `successor.effect_entry_attention().is_some()`; it is never re-derived from a record or
frontier in a backend.

### Why there is no receipt

The canonical batch is the append identity and acknowledgement marker. The run/fact after-images are
deterministic consequences of the qualified reducer step and commit atomically with it. An
additional immutable receipt would be outside all authoritative hashes and would either be ignored,
recomputed, or trusted as a second authority.

Required behavior without receipts:

- if the canonical batch is absent, compare expected current state and atomically install batch,
  objects, successor projection, and tenant-fact mutation;
- if an identical canonical batch exists and is still current, require the current projection to
  equal its compiled successor before returning `ExistingSame`;
- if that identical batch exists but later successors are current, validate that it is an exact
  ancestor and any immutable fact route is present, perform no DML, then require the store
  coordinator to load/qualify the actual current snapshot before acknowledging `ExistingSame`;
- if a conflicting batch exists under the append identity, return `AppendConflict`;
- any current projection disagreement with current canonical history is `InvalidHistory`, never
  retry repair.

Transaction atomicity means resolving an acknowledgement ambiguity by finding the exact batch proves
that its objects and co-transactional projections committed. The resolver still validates exact
batch identity and fact route when applicable. It never reapplies the in-flight transient
after-image.

Only `NewlyCommitted` may release/cache the sealed incremental `VerifiedStructuredRun`.
`ExistingSame` may name an old append while a later head is current, so it must not cache that
historical successor. For a historical retry, return current verified evidence only from the normal
qualified load; projection corruption must fail even though the retried immutable batch itself is
exact.

Replace the cache's head-only probe with `current_run_projection`. Reuse a cached run only when its
derived `RunCurrentProjection` equals the backend value from one snapshot. Same head with a different
tenant or attention membership is corruption, not a cache hit.

---

## Tenant-fact and configuration projections

### Tenant facts are an explicit mechanical command

`tenant_fact_publications` remains a disposable append-only route projection of the authoritative
`TenantFactCoordinate::FactPublication` in run history. `tenant_fact_heads` remains the mutable
cross-run sequencer/current pointer. Neither becomes a semantic stream.

The reducer emits only the unassigned `TenantFactRequirement` defined above. After the coordinator
reads the current tenant frontier and the compiler binds the exact assigned transition reference,
the compiler emits exactly one transient command:

```rust
pub enum TenantFactProjectionPlan {
    None,
    Barrier {
        expected_frontier: TenantFactFrontier,
    },
    Publish {
        expected_predecessor: TenantFactFrontier,
        publication: TenantFactPublication,
    },
}
```

The reducer establishes whether a barrier/publication is semantically required and the fact set.
The compiler binds the observed predecessor frontier, exact non-empty transition reference,
publication material, and assigned route order. Backends do not match
`TenantFactCoordinate`, `RunRecord::StateTransitionCommitted`, or facts to manufacture a command.

Mechanical rules:

- `None`: no tenant lock or tenant-fact mutation;
- `Barrier`: acquire the canonical tenant lock, require the exact frontier, mutate nothing;
- frontier zero is represented by no `tenant_fact_heads` row;
- first `Publish`: require absent/zero predecessor, insert route order 1, and create the positive
  head atomically;
- later `Publish`: require exact `N`, insert `N + 1`, and CAS the head atomically;
- old exact retries validate their immutable route but never reapply it;
- competing runs for one order yield one complete commit and one stale result without a gap.

Delete from `tenant_fact_heads`:

- `publication_count`;
- `minimum_order`;
- `maximum_order`;
- their constraints, parsers, queries, comments, and tests;
- persistent zero-head rows.

Delete `PendingTenantFactPublication`, `TenantFactRouteSummary`,
`TenantFactRouteSummary::is_dense_through`, lifetime aggregate probes, and equivalent memory/backend
record interpretation.

Semantic qualification recomputes expected commands through the reducer/compiler and proves in both
directions:

1. each authoritative fact coordinate yields the expected command;
2. each `Publish` command has one identical route and every route has one command;
3. per-tenant orders are dense from 1 and exact maxima equal positive head rows;
4. absent routes and absent head both represent zero.

Missing, invented, wrong-coordinate, duplicate, non-dense, lagging, leading, rewound, and invented
zero-head shapes fail. Deleting the latest route and rewinding the head while retaining the batch
must fail.

### Configuration uses its one shared verifier

`ValidatedConfigurationAppend` carries exact expected/successor heads. Memory and PostgreSQL compare
and apply those values mechanically; they do not reinterpret revisions.

Delete PostgreSQL's window-function configuration reducer (`ordered_configuration`,
`invalid_configuration`, and equivalent semantic SQL). Structural catalog checks load exact
revision rows; `mfm-store` invokes `verify_configuration_history` once and compares the resulting
head in both directions. Identical retries, competing successors, and acknowledgement ambiguity
retain their existing exact append semantics without a verification bit or receipt.

---

## Backend contract

### Allowed and forbidden behavior

Backends may:

- strictly decode physical row types into checked IDs and bounded canonical envelopes;
- look up an immutable append attempt by stable `(run_id, append_request_id)` without accepting
  append authority or changing state;
- compare exact append identities, predecessors, expected current projections, and supplied
  after-images;
- acquire run and tenant locks in canonical order;
- insert immutable batches, objects, configuration revisions, and fact routes;
- CAS mutable run/configuration/fact heads;
- scan the current attention membership index by tenant and run ID;
- enumerate all authoritative/projection keys for qualification.

Backends must not:

- match `RunRecord`, `StateLeaf`, `StructuredFrontier`, `AccessKind`, or `ObservationOutcome` to
  choose projection behavior;
- interpret entry modes, attempt budgets, settlement, facts, program structure, or recovery policy;
- call the run reducer, certification registry, physical verifier, or configuration verifier from DML
  methods;
- use JSON-path/window SQL as a second semantic validator;
- silently repair or fall back around disagreement.

### Snapshot and trait cutover

Replace the head-only snapshot contract with:

```text
StructuredRunSnapshot {
  history: RawRunHistory,
  current_projection: RunCurrentProjection,
}

current_run_projection(run_id)
lookup_append_attempt(run_id, append_request_id)
```

Both values come from one repeatable-read snapshot whose first decision-bearing query selects the
current projection. The store qualifies/reduces the history and compares the final derived
projection before returning evidence.

`lookup_append_attempt` is a mechanical read used to resolve a retained request before any
current-head/frontier-dependent authoring. The store—not the backend—qualifies its returned prefix
and compares the normalized recorded event with the requested intent. An absent result grants no
authority; the append transaction must classify the same key again under its lock and compare an
existing candidate exactly. Delete the redundant raw `load` backend entry point; move all callers to
`load_snapshot`. Historical prefix loading returns one exact-snapshot raw prefix and has no receipt
list. Update every backend
and fixture wrapper, including memory, PostgreSQL, positive/corrupt/injecting backends, causal-test
wrappers, and forged-route fixtures.

### Memory

Memory retains canonical batches/objects, one `RunCurrentProjection` per run, dense fact routes, and
minimal fact/config heads. Under its lock it compares the supplied expected state and installs the
supplied successors mechanically.

Delete:

- the history-wide authorization/observation discovery scan;
- `resolved`/`candidates` sets and page-local attempt reduction;
- independent tenant-fact classification;
- independent configuration semantics;
- any projection receipt store.

Discovery iterates the current run-projection map ordered by tenant and `RunId`.

---

## PostgreSQL destructive baseline

Rewrite `crates/storages/postgres/migrations/0001_store.sql`; do not add `0002`. The discarded
unpublished v7 attempt index never becomes history. Rebuild the final baseline as
`mfm.structured-run-history-postgres.v7` from the published v6 parent and reset local databases.
The schema/wire cutover and current-projection cutover land together under that one current v7.

Do not add a separate projection contract ID. The admitted store schema/reducer/compiler release
fence is the one current contract; mixed-version writers are forbidden. A future semantic projection
change rewrites the current destructive baseline under the repository's clean-slate policy.

### `run_history_batches`

Keep one immutable canonical batch row and its objects. Do not add receipt columns, receipt digests,
or a historical derived after-image. Update strict owner-local decoding for the Rust schema reset,
idempotency classification, and ambiguity resolution.

### `run_history_heads`

Add:

- `tenant_scope_id TEXT NOT NULL`;
- `has_effect_entry_attention BOOLEAN NOT NULL`.

Keep one head row per run. Add exactly one partial index:

```sql
CREATE INDEX run_history_heads_effect_entry_attention_v1
    ON run_history_heads (tenant_scope_id, run_id)
 WHERE has_effect_entry_attention;
```

The route contains only run, tenant, exact journal head, and membership. Full attention is never
persisted. Historical attempts cannot consume a page or duplicate a run.

Require `(run_id, head_sequence)` to reference the exact current batch. Qualification proves latest
batch/head correspondence in both directions and compares the complete typed current projection
with reducer output.

### Transaction order

Preserve canonical run-then-tenant advisory-lock ordering. For a new append:

1. classify `(run_id, append_request_id)` and, when present, compare the exact canonical batch
   identity/bytes;
2. lock/load the current run projection;
3. compare the full supplied expected projection;
4. validate the explicit tenant-fact precondition;
5. insert batch and exact object closure;
6. replace current head/tenant/attention membership with the supplied successor;
7. apply the supplied tenant-fact route/head command;
8. commit all surfaces atomically.

`ExistingSame` returns before mutation. If later successors exist, it validates ancestry/immutable
route and does not require the current projection to equal the historical successor.

### Delete `run_access_attempts` completely

Delete from SQL:

- the table;
- every primary/unique/foreign/check constraint;
- `run_access_attempts_possible_entry_v1` and every other attempt index;
- comments, ownership, revokes, grants, and manifest entries.

Delete from PostgreSQL Rust:

- attempt inserts and observation updates;
- `insert_access_rows`;
- kind/outcome tag helpers used only by projection;
- attempt predicate queries and record-position decoders;
- DML imports of `AccessKind`, `ObservationOutcome`, and `RunRecord`;
- access-record semantic qualification CTEs and fixtures;
- attempt-index scale-plan tests.

Replace the scale test with hundreds of real recovered histories plus one current attention run. The
partial head index must contain/visit one row, independent of retained attempts/batches.

### Two-phase qualification before exposure

Make assembly an async one-shot state transition. It consumes an unsplit, structurally qualified
backend/session bundle, the complete `QualifiedProgramRegistry`, and the admitted target/release
fence. Inside that transition only, consume the registry through `into_runtime_parts` and retain the
resulting `AdmissionVerificationRegistry`, `CertifiedProcessRegistry`, and
`RuntimeAssemblyToken` together with the unopened backend. This preserves the existing load-bearing
coupling between persisted admission trust, live process callbacks, and Runtime assembly. No caller
may pass those pieces independently or substitute one after history qualification. Offline replay
alone receives the callback-free admission snapshot and no live process registry/token.

None of those inputs can be cloned or split into a usable reader, writer, Runtime, application
backend, configuration handle, or attention reader until semantic open succeeds. Add compile-fail
coverage for every assembly/split/test-support bypass, not merely production happy paths.

Apply the one-shot path to all production openers:

- `open_structured_authoritative`: run qualification;
- `open_structured_authoritative_application`: run and configuration qualification;
- `open_structured_authoritative_with_configuration`: run and configuration qualification;
- `open_configuration_maintenance`: configuration qualification before returning its writer.

Make the in-memory assembly helper async and execute the same semantic-open path. Delete the public
synchronous `assemble_structured_runtime` bypass and any constructor that can split the opaque
bundle first and promise to qualify later.

Phase 1 establishes only a structurally usable but externally unusable session bundle:

- migration/schema identity and relation/constraint/index/executable/ACL manifests;
- exact database/schema/store target, store scope, epoch, fence generation, and release epoch;
- role shape and release-fence ownership;
- installation of bounded row decoders and raw enumeration capability.

Phase 1 performs no data-dependent row, foreign-key/orphan, batch/object, head, route, or semantic
check; doing so outside the pinned data snapshot would create a split-time authority.

Phase 2 runs all run-history work in one pinned PostgreSQL `REPEATABLE READ`, read-only transaction.
Every decision-bearing run query must use that same snapshot: the authoritative/projection key
union, raw histories and objects, current run projections, tenant-fact routes/heads, and every
recursive prior-run fact-source prefix. Do not open a nested connection/snapshot while discharging
a source obligation.

Within that snapshot:

1. enumerate the union of authoritative run IDs and current-projection run IDs in bounded pages and
   perform bounded row decoding, foreign-key/orphan integrity, and immutable batch/object physical
   correspondence inside this snapshot;
2. structurally qualify each raw history into `QualifiedHistory` and obtain its exact
   `Arc<CertifiedProgram>` through `AdmissionVerificationRegistry::verify_root`;
3. replay every `QualifiedEvent::Recorded` through `reduce_event`, compiler assertion/object
   comparison, and `RetainedOnly` discharge; `PriorRunFactCompleteness` recurses here, through the
   same snapshot and exact-prefix memo, before `FinalizedReduction`/`VerifiedStructuredRun` exists;
4. compile/compare the complete current `RunCurrentProjection` from the verified result;
5. recompute expected tenant-fact commands and compare routes/heads globally in both directions;
6. reject every missing, invented, lagging, leading, rewound, duplicate, wrong-coordinate, or
   wrong-tenant projection shape;
7. complete the read-only transaction without exposing any capability.

Use one private open-session memo for the complete snapshot:

```text
programs: (operation_id, certified_root_ref) -> Arc<CertifiedProgram>
prefixes: (run_id, through_head) -> Visiting | Verified(Arc<VerifiedStructuredRun>)
```

A `through_head` is the complete sequence-plus-digest `JournalHead`, not an ordinal alone. A request
for a longer prefix extends the greatest exact cached ancestor only after comparing that ancestor
with the retained prefix. Recursive re-entry of the same exact `Visiting` key is an invalid source
cycle; an already `Verified` exact/earlier prefix is reusable. A shared producer prefix is
semantically verified once, but every consumer request still
charges its logical producer, byte, object, fact, batch, depth, and route bounds; memoization must not
make an over-bound source graph acceptable. Cache hits for programs still compare the exact root and
authored bytes, as specified above.

Configuration uses its own pinned configuration-reader snapshot and the one shared
`verify_configuration_history`; it does not reuse PostgreSQL window/JSON semantics. Run and
configuration histories have no cross-stream semantic transition, so they do not require one
distributed database snapshot. An opener that returns both capabilities must complete both
qualifications before either can escape.

The run snapshot is the semantic-open linearization point `S0`; do not claim that no same-release
append can occur after it. The release fence forbids mixed schema/reducer/compiler writers, and every
same-release post-`S0` append must enter through `ValidatedRunAppend`, repeat commit-time target/head
validation, and atomically preserve the projection invariant. Thus the projection is exhaustively
certified at `S0` and remains valid inductively under the only writers that can run afterward.

After the required semantic snapshots finish, re-read the target authority in a fresh transaction
and require the original database identity, schema/store target, fence generation, release epoch,
and admitted schema/reducer/compiler release. If any changed, discard every qualified product and
fail open. Only that final check may consume the opaque bundle and matched process assembly token
into handles. A same-release append between `S0` and this check is allowed and must be covered by an
explicit concurrency regression; the first cache/read use compares current projection and therefore
cannot expose the stale `S0` product as current.

The in-memory opener takes one immutable snapshot under its lock, performs the same semantic audit,
then relies on the same sealed append path after releasing the lock.

For newly proposed physical material, `RetainedAndCurrent` always performs retained verification
before current verification and exact assertion comparison before constructing
`ValidatedRunAppend`. This proves that an incrementally cached successor is acceptable to the future
`RetainedOnly` replay path.

The `S0` audit is intentionally proportional to the retained authoritative histories and current
projection rows. Correct discovery completeness takes precedence over lazy startup. Add
counters/scale tests and bounded
enumeration; do not weaken the guarantee by validating only rows selected from the attention index.

Normal open never repairs. A future explicit rebuild may, under an exclusive deployment writer
fence, run the same qualification/reducer/compiler path into fresh projection tables and atomically
swap them.
No ordinary application authority receives that capability in this cutover.

Regenerate, rather than guess, the relation, constraint, index, executable/trigger, and ACL manifest
attestations in `crates/storages/postgres/src/schema.rs` against a fresh schema under the
qualification role. Remove temporary hash-printing probes. No committed SQLx compatibility data or
later migration is added.

---

## Current Effect-entry-attention inventory

### Store route and authoritative reader

Delete `PossibleEntryPosition`. The backend route is per run:

```rust
pub struct EffectEntryAttentionRoute {
    pub run_id: RunId,
    pub journal_head: JournalHead,
}
```

Replace `scan_possible_entry_routes` with:

```text
scan_effect_entry_attention_routes(
  tenant_scope_id,
  after_run_id: Option<&RunId>,
  maximum_items,
)
```

It performs only the tenant/keyset partial-index scan. For each route the store:

1. loads raw history and current projection from one snapshot;
2. skips a route whose scanned head changed before load, treating it as a live-sweep race;
3. otherwise qualifies/reduces the complete prefix and discharges retained obligations;
4. requires exact head, tenant, and attention-membership equality;
5. requires the reducer to return complete `EffectEntryAttention`;
6. emits only the existing redaction-safe evidence header, head, subject, and resolution.

Same-head disagreement is `InvalidHistory`, not an empty result. A forged tenant/index route cannot
change the tenant re-derived from the admitted run. Delete page-local deduplication: one head row per
run makes cardinality structural.

Replace the purpose-limited surface without aliases:

| Delete | Add |
| --- | --- |
| `PossibleEntryReader` | `EffectEntryAttentionReader` |
| `PossibleEntryEvidence` | `EffectEntryAttentionEvidence` |
| `PossibleEntryPageEvidence` | `EffectEntryAttentionPageEvidence` |
| `list_possible_entry_runs` | `list_effect_entry_attention` |
| `possible_entry_reader` assembly fields | `effect_entry_attention_reader` |
| attempt-position cursor fields | `next_after_run_id` |

### Pagination contract

Order strictly by `RunId`. The opaque owner-local cursor contains only:

```text
version = mfm.effect-entry-attention-cursor.v1
tenant_scope_id
after_run_id
```

The cursor is canonical, bounded, strict, tenant-bound, and rejects unknown fields/versions and old
parked/attempt cursors. It has no `SchemaId`: the cursor codec and literal version are owned by the
surface type.

Query at most `limit + 1` routes. Process only the first `limit`; the extra route is a sentinel.
When a sentinel exists, continue after the last processed route, never after the sentinel. Advance
by the last scanned/processed run, not the last emitted result. Races can therefore produce a sparse
or empty page with a non-null continuation.

This is a live level-triggered sweep, not a tenant-wide snapshot or durable change checkpoint:

- a continuously present run is emitted at most once in one sweep;
- a run entering at or behind an already-passed cursor waits for the next sweep;
- a run resolved between scan and load is omitted;
- callers restart without a cursor for another sweep;
- historical attempts never affect scan work or pagination;
- clients must follow a continuation even when the current page emits no items.

The operational detection bound is approximately:

```text
time until next full sweep
  + time to reach the run in that sweep
  + recovery drive latency
```

Document that deployment must bound sweep cadence, page latency/cardinality, and time to act. The API
alone does not create a time guarantee.

### Public one-current cutover

Use only these names:

| Delete | Replacement |
| --- | --- |
| `RunAccessGrant::ListParked` | `ListEffectEntryAttention` |
| `Application::list_parked_runs` | `list_effect_entry_attention` |
| `ParkedRun` / `ParkedRunPage` | `EffectEntryAttentionRun` / `EffectEntryAttentionRunPage` |
| `StructuredParkedRun` / `StructuredParkedRunPage` | `StructuredEffectEntryAttentionRun` / `StructuredEffectEntryAttentionRunPage` |
| `project_parked_runs` | `project_effect_entry_attention` |
| `mfm run parked` | `mfm run effect-entry-attention` |
| `GET /v1/parked-runs` | `GET /v1/effect-entry-attention` |
| `parked_occurrence` | `effect_entry_subject` |

The row carries exact run/evidence identity, journal head, `effect_entry_subject`, and one closed
resolution string:

```text
manual
close_then_reassert
reassert
```

Update the drive response to nullable `effect_entry_subject`; it remains present only for the
terminal manual block. The subject DTO is shared by drive and inventory but has no annex/schema-ID
badge. Owner-local typed codecs define exact fields, literals, bounds, canonical bytes, and
unknown-field rejection.

The grant remains distinct from drive/read/replay/audit/trace/export and targets the existing
tenant-bound `StoreTarget`. The inventory grants no drive, callback, settlement, scan, or invocation
authority. Add no CLI/REST alias, deprecated type, old cursor decoder, or fallback response field.

---

## Complete file, move, and deletion inventory

This list is a floor. The schema census must add every discovered producer/consumer; it may not
remove any item below without explaining the final owner.

### Canonical, IDs, values, facts, spec, journal, certification

Update:

- `crates/kernel/canonical/src/lib.rs` and canonical tests/README;
- `crates/kernel/ids/src/{checked.rs,identity.rs,lib.rs}`, `tests/identities.rs`, IDs README/Cargo
  manifest/lockfile;
- `crates/kernel/values/src/lib.rs`, `retained.rs`, descriptor/retained/schema tests, README;
- `crates/kernel/authority-seal/src/lib.rs` for deletion of the verifier seal/bypass surface;
- `crates/kernel/capabilities/src/lib.rs` and `crates/kernel/program/src/structured.rs` plus program
  UI/entry-mode fixtures for the final typed capability/mode edges;
- `crates/kernel/program-derive/src/lib.rs`, `shape.rs`, golden and compile tests;
- `crates/kernel/facts/src/{lib.rs,value.rs,selection.rs,emission.rs,read.rs,tests.rs}` and README;
- `crates/kernel/spec/src/{lib.rs,public.rs,structured.rs}` and their contract tests;
- `crates/kernel/journal/src/structured.rs`, tests, Cargo manifest, and README as required by
  shape-derived journal-owned types/identities;
- `crates/kernel/certify/src/{lib.rs,structured.rs}`, its public error boundary and certification
  tests;
- `crates/signing/src/lib.rs` and signing tests for owner-derived persisted descriptors.

Delete:

- the complete recoverability module/API/artifact/document list above;
- `crates/kernel/facts/src/codec.rs` if no owner-local non-registry helpers remain;
- every generic string-selected schema lookup/decoder;
- every erased `ValidatedCanonicalValue` field and recoverability-specific error conversion;
- the inner fact-selection base64 document and custom duplicate parser;
- production seed IDs for known Rust shapes, including selector, journal manifests/certificates,
  store framework objects, and configuration revisions;
- the obsolete semantic namespace `mfm.recoverability` in spec;
- schema IDs/accessors that are merely validation badges.
- public `EntryPointContract` plus its retained/decode APIs after moving output behavior to
  `PublishedEntryPoint`;
- duplicate certified-root digest/seed/object path, `certified_program_root_ref`,
  `qualified_entry_point_admission_policy_ref`, `CertifiedProgramAuditRefs`, and the admission audit
  revalidation path.

The preliminary census identifies annex-backed shape-derived survivors for component-object
evidence, fact-selection query, planning profile, portable export stream, retained-value contract,
and configuration revision. It identifies admit/drive/error/public/replay/audit/trace/frame and all
primitive/preimage IDs as codec-only or badges. Validate this disposition and the non-annex seed
sites before coding; do not infer a “40” migration.

### Store reducer/compiler/backend

Update:

- `crates/kernel/runtime/src/history/{cursor.rs,mod.rs}` and `structured.rs`;
- `crates/kernel/store/src/structured/{adapter.rs,mutation.rs,backend.rs,memory.rs,purpose.rs,assembly.rs,configuration.rs,fact_scan.rs,test_support.rs,mod.rs}`;
- add/finalize `validated_append.rs` together with qualification, reducer, compiler, obligations,
  projection, and coordinator modules;
- `crates/kernel/store/src/structured/tests.rs`;
- `crates/kernel/store/tests/{structured_runtime.rs,structured_runtime_causal.rs,api_surface.rs}`;
- test-support backends and new validated-append, reduction-typestate, and assembly-split
  compile-fail fixtures.

Delete without facade or alias:

- `crates/kernel/store/src/structured/fold.rs` after extraction;
- store-owned `VerifiedProgramData`, `ProgramVerifier`, `RegistryProgramVerifier`, their seal/build
  helpers, and permissive test verifiers;
- obsolete `program_verifier_trait_not_public` and `verified_program_data_not_constructible` UI
  fixtures/stderr after replacing them with certified-program/history and validated-append
  boundaries;
- `fold_recorded_history`, `VerifiedFoldState`, mixed `FoldMachine`, repeated recursive state lookup,
  and reducer-side component decoding;
- physical verifier arguments/calls below the obligation coordinator;
- `PhysicalBindingVerificationMode`, `ObservationQualification`, `PendingObservation`, every
  `qualify_observation`/`qualify_invoked_observation` port/adapter path, and the separate
  supersession preflight;
- `allow_generate`, generated overlays, placeholders, and duplicate transition preparation;
- `PossibleEntryPosition`, route limits/readers/evidence/page APIs, and attempt cursor fields;
- memory access/fact/config semantic reducers;
- public canonical append constructors, verification bits, and old canonical append type names;
- `crates/kernel/store/src/structured/canonical_append.rs` after callers move to
  `validated_append.rs`;
- every projection receipt concept;
- `commit_prepared`, head-only cache probes, and optional prepared successors;
- acceptance of a cached incremental successor on `ExistingSame`.

### PostgreSQL

Update:

- `crates/storages/postgres/migrations/0001_store.sql`;
- `crates/storages/postgres/src/{structured.rs,schema.rs,sql_catalog.rs,session.rs,transaction.rs,configuration.rs,qualification.rs,lib.rs}`;
- `crates/storages/postgres/tests/structured_history.rs`;
- PostgreSQL README and Nix task references.

Delete:

- `run_access_attempts` and every catalog/ACL/query/DML reference;
- all attempt image insert/update, outcome/kind tags, decoders, qualifiers, and scale fixtures;
- fact summary columns/structs/aggregates/zero rows/asymmetric qualification;
- SQL configuration-chain semantics;
- any backend import/match on journal/Runtime semantic enums for projection decisions;
- any proposed receipt columns/ACLs/selects/qualification;
- comments claiming an attempt predicate exactly equals reducer state or resolved rows disappear
  from an attempt index.

### Replay, app, CLI, REST, EVM, integration

Update:

- `crates/kernel/replay/src/{structured.rs,portable.rs}`, replay Cargo manifest/lockfile, owner
  tests, and README for the direct acyclic `mfm-certify` dependency;
- `crates/app/src/{access.rs,application.rs,production_structured.rs,surface.rs,render.rs,errors.rs,lib.rs}`;
- app README and privacy/codec/UI tests;
- `bin/cli/src/{commands/run/mod.rs,commands/ops.rs,presentation/output.rs,support/output_file.rs}`,
  CLI tests/README;
- `bin/rest-api/src/{lib.rs,tests.rs}` and REST README;
- `tests/integration/tests/{replay_wire_contract.rs,transport_surface_contract.rs,legacy_surface_contract.rs,run_identity_contract.rs,evm_postgres_submission.rs}`;
- `crates/domains/evm/src/{submission_registry.rs,wallet.rs,wallet_authority.rs,submission_tests.rs}`
  and `crates/live/evm/src/{physical_release.rs,structured.rs,structured_wallet.rs}` for every moved
  hash/limit and final absorption caller;
- `crates/storages/evm-postgres/migrations/0001_wallet_authority.sql`, schema/provider/authority
  modules, test-support provider protocol, SQLx queries, and wallet qualification fixtures for the
  destructive v2/v4 cutover;
- `crates/storages/evm-postgres/src/{support.rs,provider.rs,provider_mutation_proof_tests.rs,authority.rs}`,
  `tests/provider_protocol.rs`, `tests/wallet_authority.rs`, and
  `tests/wallet-authority-provider/src/lib.rs`;
- EVM PostgreSQL wallet authority rows/queries/provider contracts that carry redundant evidence
  references, replacing predecessor/winner selection with the existing typed keys and retained
  coordinates;
- EVM wallet/broadcast recovery behavior harvested from `b07d94207`.

Delete:

- every corpus `include_bytes!`, local corpus-vector/hex decoder, and annex-introspection test;
- old parked command/route/types/fields/schema badges/cursors;
- old possible-entry privacy UI fixture names, replacing their behavior under attention names;
- private duplicate run-ID derivations;
- `portable-replay-corpus` Nix task;
- any test expecting old annex schema IDs or old public bytes.
- `PORTABLE_FRAME_SCHEMA_CONTRACT`, every individual-frame identity field/accessor/publication,
  derivative frame ordinal/chain/final digests and repeated seal counts, and the fixed-point
  seal-size loop;
- `evidence_reference`, `mfm.evm.wallet-storage-evidence`, reservation/activation/completion/winning
  evidence refs, `predecessor_activation_ref`, `observed_floor_ref`, and
  `original_terminal_witnesses_ref`, with the direct typed replacements above and no replacement
  digest.

Add compile-fail fixtures proving:

- validated run/configuration appends cannot be constructed externally;
- `PendingSemanticStep`/`ComparedReduction` cannot expose/mix a successor or bypass whole discharge;
- admission/process registries and `RuntimeAssemblyToken` cannot be split/substituted around semantic
  open, including through test support;
- attention evidence cannot expose records;
- the attention reader is not an audit/drive reader;
- the existing sealed Effect entry-mode orphan attack still fails for the sealed bound.

### Documentation and workflow

Update current truth in the same commits that change it:

- `docs/design.md`: raw qualification, pure reducer, obligations/compiler, Rust-owned persisted
  schemas, one reset, transient projections, full semantic open qualification, attention inventory;
- `docs/architecture.md`: exact stage/package ownership and dependency direction;
- `docs/run-execution.md`: one reducer step and verified load/cache rules;
- `docs/effect-entry-resolution.md`: all three attention resolutions and honest sweep latency;
- `docs/known-gaps.md`: update the two-process coverage note and attention cadence limitation; do
  not add a schema-identity gap;
- `docs/evm-transactions.md`: actionable close/reassert versus manual attention;
- `docs/persisted-public-surfaces.md`: current retained-schema/predicate owners, bounded-shape
  vocabulary, surviving semantic-domain owner/preimage table, owner codecs, current DTOs/cursor,
  and one reset;
- `docs/build-and-verification.md`: renamed/deleted Nix leaves;
- root `README.md`, including removal of base-branch prose that describes Effect resolution as a
  proposal rather than the current contract;
- canonical/IDs/values/facts/journal/store/replay/PostgreSQL/app READMEs;
- CLI/REST READMEs and crate-level rustdocs/comments.

Delete rather than update:

- the resurrected base-branch `IMPL_PLAN_EFFECT_ENTRY_RES.md`;
- `docs/recoverability-app-surface-v1.md`;
- `docs/recoverability-predicate-owners-v1.md`;
- the already-superseded `docs/recoverability-removal.md`;
- generator/artifact READMEs.

Move still-current public-surface facts to persisted surfaces/app/CLI/REST documentation and
ownership facts to design/architecture before deletion. Leave no historical current-design prose;
Git history is the archive.

---

## Logical history rewrite

Create a private backup ref before rewriting. Rebase the branch onto `56c260ba4`, then build the
following ordered history. Do not cherry-pick the old commits wholesale: harvest their tests and
current behavior into the new owners, and omit their annex, parked-run, attempt-index, verifier-bit,
or compatibility machinery. Every commit must compile and describe one current design.

At every executable commit:

- run `nix develop -c cargo fmt --all -- --check`;
- run the narrow owner tests named for that commit;
- run `git diff --check` and inspect `git diff --stat` plus the complete diff;
- update design/architecture/current README prose in the same commit as its changed contract;
- use the lower-case subject shown below or an equally precise lower-case subject;
- do not run a broad gate merely because the commit is about to be created.

For the prose-only planning anchor, validate changed links/commands and run `git diff --check`; do
not run Rust formatting or tests merely because it will be committed.

A filtered Cargo command that reports zero tests is a failed verification selection. Prefer the
explicit targets below; when a new target is named by the plan, add it to the owning manifest and
confirm its test count before treating the command as evidence.

### Planning anchor — `plan the qualified runtime-store cutover`

Commit this plan and delete the `IMPL_PLAN_EFFECT_ENTRY_RES.md` resurrected by rebasing to
`56c260ba4`; keep `docs/recoverability-removal.md` absent. This is the sole implementation anchor and
changes no executable contract. Preserve the old branch tip only as a private local backup ref,
complete all three historical-to-final proof tables inside this anchor, amend *Material
uncertainties* to `none`, and review the amendment. If the anchor was committed as a checkpoint
before the tables were complete, amend that same commit; do not add a second planning commit. Do not
begin commit 1 while any table is incomplete or *Material uncertainties* is not `none`. Commit 2
must translate the proof into current-only persisted-surface documentation before deleting obsolete
names from the live tree.

### Commit 1 — `pin semantic hashes and reject a forged run identity`

Make the four semantic hash rules independently testable before changing any schema identity:

- move `derive_run_id` to the journal structured-identity owner and call it from admission and
  hostile genesis verification;
- add the four fixed-preimage algorithm vectors listed above at their final owners and label the
  schema-bearing fact fixtures synthetic;
- replace every hash that survives the cutover with the typed owner operation from the reviewed
  census; leave the generic helper reachable only from EVM evidence paths that commit 2 deletes, and
  add no wrapper for those doomed identities;
- replace all private run/fact derivation copies with calls to the one owner;
- add a forged-history regression where the envelope run ID and `RunAdmitted.run_id` agree with
  each other but disagree with operation, invocation, store scope, or tenant scope;
- expose raw-content hashing as an ordinary canonical byte primitive while leaving existing annex
  consumers temporarily intact;
- add no new schema formula, literal schema ID, or dual derivation.

This commit may invoke the old annex-backed domain helper inside the new owner only until commit 2;
the public call graph and goldens are already singular. Do not pin old schema IDs or portable bytes.

Focused evidence:

```bash
nix develop -c cargo test -p mfm-canonical --test recoverability_v1
nix develop -c cargo test -p mfm-journal
nix develop -c cargo test -p mfm-facts
nix develop -c cargo test -p mfm-evm
nix develop -c cargo test -p mfm-evm-live
nix develop -c cargo test -p mfm-storage-evm-postgres --lib
nix develop -c cargo test -p mfm-integration-tests --test run_identity_contract
```

The canonical recoverability target is intentionally used for the last time here and is deleted in
commit 2.

### Commit 2 — `replace recoverability with rust-owned persisted contracts`

This is one inseparable schema/wire cutover. The pre-implementation disposition/predicate tables and
`Material uncertainties: none` amendment must already be reviewed; stop rather than entering this
commit if that gate was skipped or invalidated by commit 1 findings.

In this commit:

- extend the closed `SchemaShape`/persisted-codec algebra in `mfm-values`;
- add owner-local persisted schemas, strict codecs, checked grammars, and typed `HistoryObject`
  construction/decoding for every survivor;
- cut facts to the direct typed request/query representation and the one selector engine contract;
- migrate all annex and name-seeded retained objects to their exact owners;
- cut configuration revision and certified program root to one representation each, deleting the
  duplicate admission policy/audit fields;
- delete non-retained EVM evidence `ContentRef` misuse and its redundant row/DTO fields without a
  replacement identity;
- delete the final evidence-only callers and the generic public `domain_content_digest` helper;
- redistribute all 39 limits to their owners;
- simplify portable export to the one complete-stream v3 identity, closed record union, and required
  expected-ref verification;
- cut EVM provider protocol to v4 and its wallet PostgreSQL destructive baseline to v2;
- migrate app/CLI/REST codecs, `PublishedEntryPoint`, fixtures, and current public bytes;
- add the new production-valid fact identity goldens from owner-constructed current references while
  retaining the separate fixed-preimage algorithm vectors;
- relocate all valuable corpus evidence to owner tests;
- delete the complete recoverability plane, generator, corpus, old docs, APIs, generic factories,
  central limits file, and redundant Nix leaf in the same commit;
- rename `recoverability-postgres-v1` to `structured-history-postgres-qualification`;
- replace the cutover census with current-only persisted-schema/predicate/vocabulary/semantic-hash
  owner tables in `docs/persisted-public-surfaces.md`; do not retain deleted names as a historical
  annex there;
- update every manifest/lockfile/import/document and reject the old wire/schema identities;
- make no compatibility decoder, alias, fallback, generated replacement, or second reset.

The schema identity reset is global because nested `ContentRef`s affect program roots, history
objects, batch hashes, portable streams, and fixtures. A temporary state where some producers emit
old IDs while some consumers expect new ones is not a coherent commit.

Focused evidence, selected further by the final census:

```bash
nix develop -c cargo test -p mfm-canonical --test canonical_json
nix develop -c cargo test -p mfm-ids --test identities
nix develop -c cargo test -p mfm-values
nix develop -c cargo test -p mfm-facts
nix develop -c cargo test -p mfm-journal
nix develop -c cargo test -p mfm-spec
nix develop -c cargo test -p mfm-program-derive
nix develop -c cargo test -p mfm-program
nix develop -c cargo test -p mfm-certify
nix develop -c cargo test -p mfm-signing
nix develop -c cargo test -p mfm-store --lib
nix develop -c cargo test -p mfm-replay
nix develop -c cargo test -p mfm-app
nix develop -c cargo test -p mfm
nix develop -c cargo test -p mfm-rest-api
nix develop -c cargo test -p mfm-evm
nix develop -c cargo test -p mfm-storage-evm-postgres --test provider_protocol
nix develop -c cargo test -p mfm-integration-tests --test replay_wire_contract
nix run .#model-check
nix run .#run -- --task cargo-metadata-contract
nix run .#run -- --task structured-history-postgres-qualification
nix run .#run -- --task wallet-nonce-postgres-storage-qualification
```

Run the deletion searches in *Deletion gates* before committing. Do not run the removed generator or
removed test/task names.

### Commit 3 — `replace runtime storage with three qualified layers`

This is one atomic semantic/persistence cutover. Qualification, reducer typestate, candidate
compilation, assertion comparison, obligation discharge, validated backend ingress, and mechanical
projection application are mutually dependent. Do not land a parallel interpreter, an append bridge,
or an intermediate commit in which the new coordinator still feeds a semantic backend.

In this commit:

- reuse concrete `mfm-certify::CertifiedProgram` and
  `AdmissionVerificationRegistry::verify_root`; give replay its direct concrete certification edge;
- add private `QualifiedRunContext`, `QualifiedHistory`, `QualifiedBatch`,
  `QualifiedEvent::{Intent, Recorded}`, `RecordedAssertions`,
  `ReducerState::{Unadmitted, Admitted}`, coordinate-free `PendingSemanticStep`,
  `ComparedReduction`, `FinalizedReduction`, `ReducedRunState`, and the paired
  `VerifiedStructuredRun`;
- move raw decoding, secret scanning, canonical/envelope/object/hash/predecessor/store-lineage
  checks, component closure, admission material, dynamic-value qualification, and program-schema
  correspondence out of interpretation;
- preserve every retained derived claim in `RecordedAssertions` while allowing structurally valid
  but semantically illegal history to reach the reducer;
- implement the sole `reduce_event` rule for admission and every later event, with typed artifact and
  record intents, tenant-fact requirements, and closed physical/fact obligations;
- implement early stable append-request resolution, then the non-circular absent-candidate sequence:
  intent preview, tenant-frontier read, deterministic compile/assignment, exact candidate
  requalification, recorded reduction from the same predecessor, unbound-plan equality, compiler
  assertion/object comparison into `ComparedReduction`, `RetainedAndCurrent` discharge into
  `FinalizedReduction`, and private sealing;
- implement retained replay with the same recorded reduction/compiler path and `RetainedOnly`
  discharge, plus full-prefix/incremental-prefix equality at every event;
- make the compiler the sole owner of deterministic assignment, artifact construction, retained
  assertion/object comparison, `TenantFactProjectionPlan`, and post-assignment
  `FactScanPermitSpec`;
- mint the affine `FactScanPermit` only after `NewlyCommitted`, with no permit on replay,
  `ExistingSame`, stale/conflicting/ambiguous/rejected/failed paths;
- split the sealed physical verifier into explicit retained/current operations, require retained
  before current, and prohibit `CurrentOnly` or partial discharge;
- unify Runtime mutation around `commit_event` and delete `ObservationQualification`,
  `PendingObservation`, `qualify_observation`, `qualify_invoked_observation`, the separate
  supersession preflight, and their obsolete branch/fault plumbing;
- delete `ProgramVerifier`, `RegistryProgramVerifier`, `VerifiedProgramData`, build/seal traits and
  test implementations rather than renaming them;
- delete placeholders, provisional assigned references/digests, `allow_generate`, generated
  overlays, proposal-to-record interpretation, optional prepared successors, physical verifier
  modes/callbacks below the coordinator, and all duplicate transition APIs;
- cut full load, incremental load, historical prefix, candidate, fact-source, purpose, and portable
  replay callers directly to the new owners;
- harvest `d2d467526` here: rename only `PossibleEntrySubject` to `EffectEntrySubject`, derive
  reducer-owned manual Effect attention, carry the exact occurrence/attempt/capability subject, and
  preserve legitimate `PossibleEntry` status/frontier names;
- replace verification-bit canonical appends with `ValidatedRunAppend` and
  `ValidatedConfigurationAppend`; compile expected/successor run projections and tenant-fact plans
  only from `FinalizedReduction`;
- cut backend traits and memory/PostgreSQL to mechanical lookup/compare/insert/CAS/apply behavior;
  delete backend Runtime/fact/configuration reducers, raw load, fact summaries, and every receipt;
- rewrite the sole PostgreSQL destructive v7 baseline, delete `run_access_attempts`, extend current
  run heads with tenant/manual-attention membership, and add the one current-attention partial index;
- implement exact stable-request retry, historical ancestor, race, and acknowledgement behavior
  without recompiling retained tenant coordinates or reapplying after-images;
- make all four production openers and in-memory assembly async one-shot paths that retain the full
  program/process/token coupling, certify snapshot `S0` with one exact-prefix memo, qualify
  configuration through its shared verifier, and freshly revalidate the admitted release before
  exposing capabilities;
- regenerate SQL/ACL/index manifest attestations and SQLx metadata by the repository-owned process;
- add compile-fail coverage for validated append construction, provisional typestate, and assembly
  splitting/substitution;
- delete `fold.rs`, `fold_recorded_history`, `VerifiedFoldState`, mixed `FoldMachine`, separate
  `verify_actionable_history`, repeated component walks, obsolete re-exports, and superseded UI
  fixtures in this same commit;
- update `docs/design.md`, `docs/architecture.md`, `docs/run-execution.md`, store/PostgreSQL/app
  READMEs, and current Effect-resolution prose to state the final three-layer ownership.

No workflow decision moves into qualification. No provisional intent result can be sealed. No raw
retained container enters `ReducedRunState`. There is exactly one semantic transition definition in
the final tree: `reduce_event`.

Focused evidence:

```bash
nix develop -c cargo test -p mfm-certify
nix develop -c cargo test -p mfm-store --lib
nix develop -c cargo test -p mfm-store --features backend-conformance --test structured-runtime
nix develop -c cargo test -p mfm-store --features test-support --test structured-runtime-causal
nix develop -c cargo test -p mfm-store --test api-surface
nix develop -c cargo test -p mfm-replay --lib
nix develop -c cargo test -p mfm-app
nix develop -c cargo test -p mfm
nix develop -c cargo test -p mfm-rest-api
nix develop -c cargo test -p mfm-integration-tests --test replay_wire_contract
nix run .#run -- --task cargo-metadata-contract
nix run .#run -- --task postgres-sql-inventory-check
nix run .#run -- --task postgres-sqlx-check
nix run .#run -- --task structured-history-postgres-qualification
```

Run the reducer/compiler/qualification source gates, fold/verifier/observation deletion gates, and
construction-boundary compile-fail tests before committing.

Memory conformance is part of `mfm-store`; do not invent a separate memory package or task. Because
this is the primary database cutover, the renamed structured-history target must include the hostile
qualification matrix, not merely happy-path append tests.

### Commit 4 — `recover a crashed read attempt without a waiting frontier`

Harvest the Read behavior of `20e27bd00`:

- remove the obsolete waiting-reads frontier/path;
- keep Read recovery deterministic under the reducer's actionable state;
- preserve Read-versus-Effect entry rules and exact attempt chronology;
- express any physical/currentness work only as the obligations introduced in commit 3;
- update semantic tests and current docs, with no attention entry for Read.

Focused evidence:

```bash
nix develop -c cargo test -p mfm-store --lib
nix develop -c cargo test -p mfm-store --features test-support --test structured-runtime
nix develop -c cargo test -p mfm-replay --lib
```

The selected tests must report the Read authorization/crash/recovery/observation/settlement/
reassertion cases and full/incremental equality at each prefix; zero matching semantic cases is not
evidence.

### Commit 5 — `seal the effect entry mode`

Harvest `8379fd6e6`:

- keep `EffectEntryModeFor<Req>` sealed and request-specific;
- preserve the exact equality guards for the three access kinds;
- update current adapters/operations to the one mode API and delete superseded mode paths;
- retain the compile-fail orphan attack and assert that rustc fails on the sealed trait bound;
- update Cargo metadata/taxonomy if crate edges changed.

Focused evidence:

```bash
nix develop -c cargo test -p mfm-program --test structured-entry-mode
nix develop -c cargo test -p mfm-program --test structured-ui
nix develop -c cargo test -p mfm-app --test application-privacy-ui
nix run .#run -- --task cargo-metadata-contract
```

### Commit 6 — `close and reassert effect entry attention`

Harvest `e1db19543` into the reducer-owned attention model:

- implement `Manual`, `CloseThenReassert`, and `Reassert` exactly as the table above;
- make `Reassertable` carry the immediately preceding attempt ID;
- close an absorbing attempt with the kernel closure adapter before reassertion;
- preserve the existing minimum-lineage-head and previous-physical-binding rules; do not relax
  either to make closing convenient;
- stop exactly at the declared entry budget;
- retain the named authorize/crash/close/reassert/reassert/budget-exhaustion regressions and inspect
  every audit record for the kernel closure discriminator;
- race two workers committing the same closure and prove one commit plus one qualified reload/
  `ExistingSame`, with no component-attributed `CandidateRejected` fault noise;
- do not add the public tenant inventory or a route table in this commit.

Focused evidence:

```bash
nix develop -c cargo test -p mfm-store --lib
nix develop -c cargo test -p mfm-store --features test-support --test structured-runtime
nix develop -c cargo test -p mfm-store --features test-support --test structured-runtime-causal
nix develop -c cargo test -p mfm-app
```

Confirm the output includes closure/reassertion, kernel adapter discriminator, lineage, race, and
full/incremental-prefix cases; do not accept a broad package command that silently omits them.

### Commit 7 — `list current effect entry attention by run`

Harvest only the public intent of `b61a89c7c`, not its attempt-index implementation:

- add the per-run attention route/reader and live run-key cursor;
- add the tenant grant, app method, typed DTOs, rendering, CLI command, and REST endpoint under the
  one current `effect-entry-attention` names;
- return `manual`, `close_then_reassert`, or `reassert` from authoritative qualified/reduced evidence;
- prove tenant isolation, no authority amplification, route-not-answer behavior, live-page races,
  sparse/empty page continuation, and at-most-once-per-sweep ordering;
- replace the old scale fixture with many fully recovered histories plus one current attention run
  and prove index work depends on current membership, not retained lifetime attempts;
- delete parked names, old cursors, aliases, page-local deduplication, and false latency claims.

Focused evidence:

```bash
nix develop -c cargo test -p mfm-store --lib
nix develop -c cargo test -p mfm-store --features test-support --test structured-runtime
nix develop -c cargo test -p mfm-store --test api-surface
nix develop -c cargo test -p mfm-app
nix develop -c cargo test -p mfm
nix develop -c cargo test -p mfm-rest-api
nix run .#run -- --task structured-history-postgres-qualification
```

Confirm these targets exercise purpose/pagination, partial-index query plan, privacy, JSON/text,
REST route/schema, and grant-boundary cases with nonzero counts.

### Commit 8 — `resolve evm effect attention by absorption`

Harvest `b07d94207` against the final interfaces:

- change only the EVM wallet/broadcast adapter behavior needed to close/reassert absorption;
- keep `Ok(None) | Err(_) => EntryUnknown`; absence never becomes a non-entry claim;
- cover manual, close/reassert, budget exhaustion, restart, and two-process limits honestly;
- update EVM docs/known gaps and remove obsolete claims;
- run every final deletion and acceptance gate;
- delete this implementation plan in this commit after all gates except the post-deletion searches;
- rerun post-deletion searches without excluding the plan;
- run the final `nix run .#ci` exactly once on the final tree.

Focused evidence before the final gate:

```bash
nix develop -c cargo test -p mfm-evm
nix develop -c cargo test -p mfm-evm-live
nix develop -c cargo test -p mfm-app
nix run .#run -- --task evm-postgres-submission-qualification
```

Do not require an empty EVM-PostgreSQL diff: the limit move and deletion of false
evidence-reference fields legitimately change it.

### Old-commit disposition

| Old local commit | Disposition |
| --- | --- |
| `d2d467526` | behavior/tests harvested in commit 3; old subject/schema implementation discarded |
| `b61a89c7c` | public inventory intent harvested in commit 7; attempt table/index/pagination discarded |
| `20e27bd00` | Read semantics/tests harvested in commit 4 against the reducer |
| `8379fd6e6` | sealed entry-mode contract/tests harvested in commit 5 |
| `e1db19543` | closure/reassertion semantics/tests harvested in commit 6 |
| `b07d94207` | EVM adapter behavior/tests harvested in commit 8 |
| `254e152d2`, `545981f10` | superseded planning/removal prose replaced by the planning anchor |

Do not preserve the old commit count as an acceptance condition. The new history is organized by
responsibility and each commit's coherent current design.

---

## Required regression matrix

Names may follow local conventions, but every row is required. Prefer table-driven owner tests and
small hostile mutations over one enormous integration fixture. Full-history cases must be built from
current typed owners, not frozen annex/corpus bytes.

### Persisted schema, codec, and identity

- Every surviving persisted owner round-trips its exact canonical bytes and derived `SchemaId`.
- Changing a field, tag, literal, bound, grammar, number profile, referenced shape, framing rule, or
  manual semantic version changes the derived ID.
- Changing `SchemaAudit` provenance alone does not change it.
- Unknown fields, duplicate keys, floats, non-canonical number spelling/key order, invalid UTF-8,
  invalid ID/media grammar, bad base64url, wrong literal, and every lower/upper bound fail at the
  owner boundary.
- Fact subject/predicate open JSON rejects signed values and forbidden scalar forms while the
  explicitly general profile retains its stated signed-integer behavior.
- Generic typed `HistoryObject` decode rejects the right bytes under the wrong schema or object
  kind; the typed constructor supplies its owner object kind and production cannot construct an
  arbitrary name-seeded/string-kind object.
- Old annex schema IDs, old configuration identity, old fact request wrapper, old portable stream,
  and old DTO/cursor fields fail directly after the reset.
- The one complete portable stream codec rejects omitted, extra, reordered, substituted,
  duplicate, stale-head, false-frontier/publication, identity-collision, cyclic-source, over-bound,
  semantic-suffix, audit-suffix, and forged observed-Read frames.
- Portable v3 encoding returns bytes and their exact `ContentRef` together; offline decode rejects a
  wrong expected ref before semantic replay, rejects v2, and contains none of the deleted ordinal,
  frame-chain/final digest, repeated count, or fixed-point seal machinery.
- The four fixed-preimage algorithm fixtures equal the pinned goldens; separately, current
  owner-constructed fact fixtures equal their new post-reset goldens. No test claims nested
  `ContentRef` schema IDs were removed from a fact preimage.

### Qualification

- Wrong run/store/tenant/epoch/predecessor/sequence/candidate/commit/assigned-record/object digest
  fails before reduction.
- Duplicate logical keys, missing directly referenced bytes, object-type mismatch, hidden retained
  reference, incomplete certified component closure, and mismatched authored/certified program fail
  qualification.
- A structurally valid missing/extra *semantic introduced-object suffix* passes qualification and
  fails the compiler's exact assertion/object-closure comparison, proving that qualification is not
  a workflow validator.
- An admission/envelope pair that shares one forged run ID but disagrees with the shared derivation
  fails.
- Live and offline program sources qualify the same exact program into equivalent immutable indexes;
  a nominally equal root with different trusted bytes cannot hit a cache.
- Qualification accepts a structurally valid but semantically illegal record sequence and the
  reducer rejects it, proving the responsibility boundary.
- No public/test-only constructor can counterfeit certification, construct `QualifiedHistory` or
  `QualifiedBatch`, discharge obligations, seal `VerifiedStructuredRun`, or mint a validated append.

### Reducer and compiler

For every prefix, compare fresh full recorded reduction/compare/discharge with one incremental
recorded event through the same `reduce_event`. Cover:

- admission with zero states and immediate close;
- Pure success and typed failure;
- declaration order, lexical binding, match, fragment, fan-out, join, and failure routing;
- Read authorize/crash/observe/settle/reassert;
- Effect authorize/unknown/manual, refresh/supersession, close/reassert, and budget exhaustion;
- facts, publication, selection barrier, source completeness, and terminal closure;
- illegal adjacent/non-adjacent `RunClosed`, duplicate outstanding access, wrong ordinal, wrong
  subject, wrong observation, and impossible execution-kind/leaf combinations;
- current cursor/frontier, semantic and physical heads, bindings, terminal outcome, attention,
  obligations, expected records/artifacts/closure, fact plan, and run projection.

For each candidate class, independently mutate one retained assertion—record reference, semantic
head, fact identity, object bytes/reference, object closure, transition outcome, or assigned
coordinate—and prove retained replay rejects it. Candidate compilation followed by qualification and
recorded reduction must equal replay of the committed bytes. For every event family, also prove:

- intent preview and requalified recorded reduction return the same semantic plan;
- the coordinate-free unbound successor cannot be used as `ReducedRunState`; no intent preview can
  construct a verified run, validated append, cache entry, or fact permit;
- omitted, altered, reordered, or substituted `RecordedAssertions` fail before sealing;
- only the consuming `PendingSemanticStep -> ComparedReduction -> FinalizedReduction` typestate
  reaches a bound successor, and comparison/discharge fragments cannot be mixed across steps;
- `QualifiedHistory` retains the evidence while `ReducedRunState` contains no raw/assigned
  container.

### Obligations

- Physical authorization binds the exact attempt/certificate, kind, capability, adapter,
  implementations, route, lineage, minimum head, previous binding, and evidence.
- Supersession binds the exact authorized binding, semantic/adapter tuple, lineage/head, and evidence.
- Changing or omitting any bound field rejects without letting a verifier supply alternate state.
- `RetainedAndCurrent` always invokes retained verification first and current verification second;
  the cached successor must reload under `RetainedOnly`.
- Historically valid material that is no longer current replays under `RetainedOnly` but cannot be
  newly proposed; current-looking material that fails retained verification is rejected.
- No partial discharge or intent-preview discharge product can escape or satisfy sealing.
- A stable retained-request hit performs only the ordinary `RetainedOnly` replay/current reload; it
  performs no `RetainedAndCurrent` candidate discharge or mutation. A genuinely absent successor
  does.
- Live and portable fact verification recompute the identical request at the identical captured
  frontier. A well-formed but false positive response, missing source, invented route, incomplete
  closure, cycle, or over-budget graph fails in both paths.
- No `FactScanPermitSpec` exists before tenant-frontier resolution and exact batch assignment. A
  fact-scan permit is issued once only after the reserved authorization is `NewlyCommitted`, binds
  the exact request, authorization ref, predecessor/successor heads, frontier, source manifest,
  scanner binding, physical binding, and work bounds, and is absent for `ExistingSame`, replay,
  failed/stale append, acknowledgement-unknown resolution, non-reserved authorization, or a
  caller-supplied request.
- Unavailable external evidence maps to the reviewed typed fault and cannot create a reducer branch
  or fabricated negative fact/entry answer.

### Effect-entry attention

- Manual attention exists for an Effect possible entry with no remaining absorption.
- `EntryClosable` yields `CloseThenReassert` even though the continuation frontier is `Actions`.
- Effect `Reassertable` yields `Reassert` naming the exact immediately preceding attempt.
- Read `Reassertable` never enters Effect attention.
- Authorize, crash, close, reassert twice, and stop at the third/budget-exhausted attempt is retained
  as a named regression.
- Every budget-exhaustion audit record proves the kernel closure adapter discriminator.
- Two workers racing the same closing observation produce one append; the loser reloads the
  authoritative successor without a component-attributed `CandidateRejected` fault.
- The unchanged minimum-lineage-head and previous-physical-binding rules still produce `None` for
  `Reassertable` and their regression expectations remain explicit.

### Mechanical backend and idempotency

Run every backend conformance case against memory and PostgreSQL:

- new append atomically installs batch, objects, current run projection, and fact command;
- stale expected run or tenant head changes nothing;
- object/batch/fact/head failure rolls back every surface;
- exact current retry returns `ExistingSame` and performs no DML;
- exact historical ancestor retry returns `ExistingSame`, proves immutable ancestry/route, and does
  not rewind a current projection or cache its historical successor; the coordinator qualifies the
  actual current snapshot and rejects a corrupt current projection before acknowledgment;
- acknowledgement loss after a fact barrier at frontier `N`, followed by another run publishing
  frontier `N + 1`, still resolves the stable old append request as `ExistingSame` without
  recompiling it at `N + 1`; the same request ID with a different normalized intent is conflict;
- an absent prelookup raced by the identical append is reclassified under the append locks, returns
  `ExistingSame`, and releases no fact permit or historical cache entry;
- conflicting batch returns `AppendConflict`;
- acknowledgement ambiguity resolves through exact committed batch identity and never reapplies an
  after-image;
- same journal head with wrong tenant or attention bit is corruption, not a cache hit;
- configuration retries/CAS follow the same sealed/mechanical rule;
- concurrent run successors serialize, and concurrent tenant publications yield one dense order
  with no partial batch or gap.

Compile-fail tests prove downstream crates can name but cannot construct validated run/configuration
appends or extract a batch while silently dropping its required projection plan.

### Projection qualification and corruption

On open, reject each mutation independently:

- missing, invented, lagging, leading, rewound, wrong-tenant, wrong-head, or wrong-attention run head;
- a canonical run missing from the head table and a head without a canonical run;
- missing, invented, duplicate, non-dense, wrong-coordinate, wrong-order, or wrong-tenant fact route;
- missing/invented/rewound fact head, including an invented persistent zero head;
- configuration revision/head mismatch in either direction;
- corrupt retained history even when its current projection happens to look correct;
- a missing attention row for a reducer-positive run and an invented row for a reducer-negative run.

Prove that no reader/writer/inventory/configuration handle escapes between structural and semantic
open phases. A shared exact producer prefix is verified once, exact-prefix `Visiting` cycles fail,
verified earlier prefixes can be reused/extended, memo hits cannot bypass per-request work bounds,
configuration failure blocks combined openers, and a changed database/fence/release identity at the
final fresh check fails open. A same-release sealed append between `S0` and that check is accepted by
the inductive contract, cannot make an `S0` cache entry appear current, and leaves the projection
valid. Normal open must not modify any corrupted surface.

### Attention discovery and pagination

- A forged route is not evidence: tenant and attention are re-derived from admitted qualified history.
- A recovered `EntryUnknown -> Returned -> Closed` run consumes no route.
- A run with arbitrarily many historical unknown/reasserted attempts consumes exactly one route only
  while currently attentive and is emitted once per sweep.
- Hundreds of recovered runs plus one current attentive run visit one partial-index row; the query
  plan uses `run_history_heads_effect_entry_attention_v1`.
- `limit = 1` across multiple attentive runs has stable increasing `RunId` order, no duplicate, no
  skip in a quiescent sweep, and the correct sentinel-derived continuation.
- A resolution/head change between scan and load is skipped; a same-head mismatch is corruption.
- An entry created behind the cursor waits until the next sweep; an empty/sparse page can carry a
  continuation; restarting without a cursor rediscovers an unresolved run.
- Cursor version, tenant binding, canonical encoding, size, unknown fields, and old parked/attempt
  shapes are hostile-tested.

### Public/replay/EVM boundaries

- App/CLI/REST expose only the current attention names and the three closed resolution spellings.
- The subject contains no record payload, private implementation identity, certificate bytes,
  capability to drive, or cross-tenant data.
- The list grant cannot read/audit/drive/export/replay and those grants cannot list by accident.
- Text and JSON outputs share the app-owned DTO; CLI and REST error envelopes remain byte/field
  consistent and redaction-safe.
- Online and offline replay agree on current semantic result/bytes without asserting obsolete annex
  badges.
- EVM retains `Ok(None) | Err(_) => EntryUnknown`, closes through the kernel adapter when allowed,
  reasserts only within budget, and resumes correctly after process/database restart.
- EVM wallet provider v4/PostgreSQL v2 reject old JSON and detect direct tampering of reservation,
  candidate/completion operation keys, state inputs/results, observed floor, prefix winner, terminal
  witnesses, and signed provider mutation without any evidence-reference field.
- The known two-process seam limitation remains explicit until the harness can drive inside one
  in-flight absorption window; it is not presented as covered.

---

## Deletion and ownership gates

Run these before deleting this plan with the plan exclusion. In commit 8, delete the plan and rerun
the applicable searches without that exclusion. Run the zero-result blocks in one Bash session with
the helper below so the first match or `rg` execution error stops the gate; a bare `rg` exit 1 is not
itself a successful shell command. General searches cover tracked/non-ignored files, including hidden
files, while excluding `.git` and build outputs. Do not use blanket `--no-ignore`, which could print
local secrets; scan only known-safe ignored generated paths such as `.sqlx` explicitly. Run the later
review-only searches in their own strict Bash sessions because they intentionally print an allowlist
for human inspection; each such block repeats its shell mode and helpers so no result is masked by a
later successful command.

```bash
set -euo pipefail

expect_no_matches() {
  local rg_exit
  if rg "$@"; then
    return 1
  else
    rg_exit=$?
    test "$rg_exit" -eq 1
  fi
}

expect_no_tracked_matches() {
  local git_grep_exit
  if git grep "$@"; then
    return 1
  else
    git_grep_exit=$?
    test "$git_grep_exit" -eq 1
  fi
}
```

### Recoverability plane: zero results

```bash
test ! -e IMPL_PLAN_EFFECT_ENTRY_RES.md

expect_no_matches -n -i \
  'RecoverabilityContract|RecoverabilityError|ValidatedCanonicalValue|CanonicalReferencePath|ReferenceTerminalKind|SchemaReferenceEdge' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'

expect_no_matches -n -i \
  'contracts/recoverability|recoverability_limits|recoverability-postgres-v1|portable-replay-corpus' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'

expect_no_matches -n -i \
  '\brecoverability\b|recoverability[-_]|mfm[.]recoverability|\bannex\b' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'

expect_no_matches -n \
  'PORTABLE_(ARTIFACT|OFFLINE_ACCEPTANCE|STORE_OBSERVED_READ_AUDIT)_HEX|ANNEX_BYTES|EMBEDDED_CONTRACT' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'

rg --files --hidden --glob '!.git/**' --glob '!target/**' \
  | expect_no_matches 'recoverability|contracts/recoverability'
```

After the plan is deleted, the final file gate and all four content gates run without an exclusion.
Do not waive a documentation/test hit as harmless; current prose and obsolete test names are part of
the removed plane.

### Monolithic fold and duplicate interpretation: zero results

```bash
test ! -e crates/kernel/store/src/structured/fold.rs
test ! -e crates/kernel/store/src/structured/canonical_append.rs
test ! -e crates/kernel/store/tests/ui/fail/program_verifier_trait_not_public.rs
test ! -e crates/kernel/store/tests/ui/fail/program_verifier_trait_not_public.stderr
test ! -e crates/kernel/store/tests/ui/fail/verified_program_data_not_constructible.rs
test ! -e crates/kernel/store/tests/ui/fail/verified_program_data_not_constructible.stderr

expect_no_matches -n \
  'fold_recorded_history|VerifiedFoldState|\bProgramVerifier\b|\bRegistryProgramVerifier\b|ProgramVerifierSeal|VerifiedProgramData|build_program_verifier|\bQualifiedProgram\b|\bProgramQualifier\b|\bPendingReduction\b|\bPhysicalBindingVerification\b|allow_generate|generated_objects|placeholder_semantic_digest|placeholder_record_ref|verify_actionable_history|framework_schema_id' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'

expect_no_matches -n \
  'dyn (ProgramVerifier|PublicPhysicalBindingVerifier)|PhysicalBindingVerificationMode|\bRetainedHistory\b|\bCurrentCandidate\b|ObservationQualification|qualify_observation|qualify_invoked_observation|PendingObservation|CanonicalRunAppend|CanonicalConfigurationAppend|store_verified|is_store_verified' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'

expect_no_matches -n \
  'commit_prepared|prepare_admission|prepare_authorization|prepare_observation|prepare_state_transition|RunReducer::(start|prepare|finalize)' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'
```

The final `reducer.rs` must also pass this zero-result boundary gate:

```bash
expect_no_matches -n \
  'CommittedBatch|AssignedRecord|HistoryObject|RawRunHistory|RunRecord|RecordedAssertions|serde_json|dyn |async |await|impl Future|Future<|FnOnce|FnMut|Fn\(|Verifier|Backend|Postgres|sqlx|std::fs|tokio::fs|reqwest|Runtime.*Proposal|CanonicalRunAppend|ValidatedRunAppend|RunProjectionReceipt|FactScanPermitSpec|TenantFactProjectionPlan|allow_generate|generated_objects|PhysicalBindingVerificationMode' \
  crates/kernel/store/src/structured/reducer.rs

expect_no_matches -n \
  'StateLeaf::|StructuredFrontier::|ActionableState|walk_program|find_state|decode_component' \
  crates/kernel/store/src/structured/compiler.rs \
  crates/kernel/store/src/structured/projection.rs

expect_no_matches -n \
  'StateLeaf|StructuredFrontier|ActionableState|ReducedRunState|FoldMachine|walk_program|find_state' \
  crates/kernel/store/src/structured/qualification.rs
```

This grep is necessary but cannot prove the absence of a renamed statically dispatched capability.
Review every reducer import, generic parameter, function argument, and return type against the
allowlist of qualified values, pure reducer types, typed artifacts, and obligation descriptions. If
a harmless word appears in a test comment, rewrite the comment to state the positive reducer contract
instead of weakening the gate. Review compiler/projection hits conceptually as well: those modules
may copy a reducer-owned result into bytes/after-images, but may not match semantic leaves to choose
one. Qualification may decode and structurally normalize `RunRecord`; it may not decide current-leaf
legality.

### Receipt and historical after-image proposals: zero results

```bash
expect_no_matches -n \
  'RunProjectionReceipt|ProjectionReceipt|projection_receipt|receipt_digest|receipt_bytes|projection_contract_id' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'
```

This gate is deliberately separate: no implementation should resurrect a receipt while deleting
the old fold or attempt table.

### False and duplicate identities: zero results

```bash
expect_no_matches -n \
  'PORTABLE_FRAME_SCHEMA_CONTRACT|previous_frame_digest|final_frame_digest|frame_chain_digest|fixed.point.*seal|certified_program_root_ref|CertifiedProgramAuditRefs|validate_admission_audit_refs|qualified_entry_point_admission_policy_ref' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'

expect_no_matches -n \
  'evidence_reference|mfm[.]evm[.]wallet-storage-evidence|reservation_evidence_ref|activation_evidence_ref|completion_evidence_ref|winning_activation_evidence_ref|predecessor_activation_ref|observed_floor_ref|original_terminal_witnesses_ref' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'

expect_no_matches -n \
  'mfm[.]evm[.]wallet-authority-postgres[.]v1|PROTOCOL_VERSION: u16 = 3' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'

expect_no_matches -n \
  '\bEntryPointContract\b|mfm[.]certified-program[.]v1|mfm[.]structured-certified-program-root|mfm[.]portable-run-export-frame|mfm[.]entry-point-contract[.]v1' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'
```

The final portable source may still use local variables for the number/bytes accumulated while
enforcing bounds; it must not serialize repeated `total_frames`/`total_bytes` assertions into the
seal. Review those two generic field names separately rather than weakening the exact derivative
field gate. The retained internal `mfm.structured-entry-point-contract` is intentionally distinct
and is not matched by the public `mfm.entry-point-contract.v1` gate.

### Attempt route and old public inventory: zero results

```bash
expect_no_matches -n \
  'run_access_attempts|run_access_attempts_possible_entry|insert_access_rows|scan_possible_entry_routes|PossibleEntryPosition|PossibleEntryReader|PossibleEntryEvidence|PossibleEntryPageEvidence|MAX_POSSIBLE_ENTRY_ROUTES|possible_entry_reader|list_possible_entry_runs' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'

expect_no_matches -n \
  'list_parked_runs|ListParked|ParkedRun|parked_occurrence|/v1/parked-runs|run parked|PossibleEntrySubject' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'

rg --files --hidden --glob '!.git/**' --glob '!target/**' \
  | expect_no_matches -i 'possible_entry_(reader|evidence|page)|parked_(run|occurrence)'
```

Do not search for or rename `PossibleEntry`, `EntryUnknown`, or conceptual parked-attempt discussion;
those remain legitimate semantic terms as specified above.

### Tracked-content complement: zero obsolete contracts

`rg` deliberately respects ignore files so it can inspect non-ignored working-tree additions without
printing arbitrary local generated material. A force-tracked ignored file could otherwise escape.
Mirror every repository-wide forbidden family above through `git grep`, which safely covers the
complete tracked working tree, including force-tracked ignored paths. Any addition to an earlier
forbidden pattern must be added to this array in the same change.

```bash
tracked_forbidden_patterns=(
  'RecoverabilityContract|RecoverabilityError|ValidatedCanonicalValue|CanonicalReferencePath|ReferenceTerminalKind|SchemaReferenceEdge'
  'contracts/recoverability|recoverability|recoverability_limits|recoverability-postgres-v1|portable-replay-corpus|mfm[.]recoverability|annex'
  'PORTABLE_(ARTIFACT|OFFLINE_ACCEPTANCE|STORE_OBSERVED_READ_AUDIT)_HEX|ANNEX_BYTES|EMBEDDED_CONTRACT'
  'fold_recorded_history|VerifiedFoldState|ProgramVerifier|RegistryProgramVerifier|ProgramVerifierSeal|VerifiedProgramData|build_program_verifier|(^|[^[:alnum:]_])QualifiedProgram([^[:alnum:]_]|$)|(^|[^[:alnum:]_])ProgramQualifier([^[:alnum:]_]|$)|(^|[^[:alnum:]_])PendingReduction([^[:alnum:]_]|$)|PhysicalBindingVerification|allow_generate|generated_objects|placeholder_semantic_digest|placeholder_record_ref|verify_actionable_history|framework_schema_id'
  'PublicPhysicalBindingVerifier|RetainedHistory|CurrentCandidate|ObservationQualification|qualify_observation|qualify_invoked_observation|PendingObservation|CanonicalRunAppend|CanonicalConfigurationAppend|store_verified|is_store_verified'
  'commit_prepared|prepare_admission|prepare_authorization|prepare_observation|prepare_state_transition|RunReducer::(start|prepare|finalize)'
  'RunProjectionReceipt|ProjectionReceipt|projection_receipt|receipt_digest|receipt_bytes|projection_contract_id'
  'PORTABLE_FRAME_SCHEMA_CONTRACT|previous_frame_digest|final_frame_digest|frame_chain_digest|fixed.point.*seal|certified_program_root_ref|CertifiedProgramAuditRefs|validate_admission_audit_refs|qualified_entry_point_admission_policy_ref'
  'evidence_reference|mfm[.]evm[.]wallet-storage-evidence|reservation_evidence_ref|activation_evidence_ref|completion_evidence_ref|winning_activation_evidence_ref|predecessor_activation_ref|observed_floor_ref|original_terminal_witnesses_ref'
  'mfm[.]evm[.]wallet-authority-postgres[.]v1|PROTOCOL_VERSION: u16 = 3'
  '(^|[^[:alnum:]_])EntryPointContract([^[:alnum:]_]|$)|mfm[.]certified-program[.]v1|mfm[.]structured-certified-program-root|mfm[.]portable-run-export-frame|mfm[.]entry-point-contract[.]v1'
  'run_access_attempts|run_access_attempts_possible_entry|insert_access_rows|scan_possible_entry_routes|PossibleEntryPosition|PossibleEntryReader|PossibleEntryEvidence|PossibleEntryPageEvidence|MAX_POSSIBLE_ENTRY_ROUTES|possible_entry_reader|list_possible_entry_runs'
  'list_parked_runs|ListParked|ParkedRun|parked_occurrence|/v1/parked-runs|run parked|PossibleEntrySubject'
)

for tracked_forbidden_pattern in "${tracked_forbidden_patterns[@]}"; do
  expect_no_tracked_matches -n -I -i -E "$tracked_forbidden_pattern" -- \
    . ':!IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md'
done

git ls-files | expect_no_matches -i \
  'recoverability|contracts/recoverability|possible_entry_(reader|evidence|page)|parked_(run|occurrence)|program_verifier_trait_not_public|verified_program_data_not_constructible'
```

After commit 8 deletes this plan, rerun this complement without the negative plan pathspec. This
tracked pass and the earlier `rg` pass are both required: together they cover modified tracked files,
force-tracked ignored files, and non-ignored untracked files without scanning arbitrary ignored
secrets.

### Known-safe ignored SQL metadata: zero obsolete contracts

Do not scan arbitrary ignored files. If repository-owned SQLx metadata is introduced, scan only its
known directories. Commit 2 must derive this allowlist from the final `nixfied.nix` SQLx tasks,
workspace Cargo configuration, and tracked SQLx metadata paths; compare it with
`git check-ignore --no-index -v` on those exact candidate paths. The three paths below are the
current candidates. Add any newly configured repository-owned SQLx directory explicitly before
commit 3; never replace the list with an ignored-tree glob.

```bash
for safe_generated_path in \
  .sqlx \
  crates/storages/postgres/.sqlx \
  crates/storages/evm-postgres/.sqlx
do
  if test -d "$safe_generated_path"; then
    for tracked_forbidden_pattern in "${tracked_forbidden_patterns[@]}"; do
      expect_no_matches -n -i "$tracked_forbidden_pattern" \
        "$safe_generated_path" --hidden --no-ignore
    done
  fi
done
```

### Seed/factory and identity census: reviewed to an explicit allowlist

```bash
set -euo pipefail

expect_no_matches() {
  local rg_exit
  if rg "$@"; then
    return 1
  else
    rg_exit=$?
    test "$rg_exit" -eq 1
  fi
}

review_rg() {
  local review_exit=0
  rg "$@" || review_exit=$?
  test "$review_exit" -le 1
}

expect_no_matches -n \
  'SCHEMA_SEED|mfm[.]structured-schema[.]v1|structured_content_ref|typed_content_ref|wallet_descriptor_ref|framework_object|fixed_schema_id|domain_content_digest' \
  crates --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

review_rg -n 'Schema(Id|Identity)::(new|parse|from_str)|ContentRef::new|schema_id\(' \
  crates --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

review_rg -n \
  'mfm[.][a-z0-9_.-]+[.]v[0-9]+|domain.*canonical|canonical.*domain' \
  crates --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

review_rg -n \
  'mfm_canonical::(recoverability|limits)\b|MAX_(PORTABLE|FACT|PROVIDER|COMPLETION|CONFIGURATION|BATCH|STORED|PRIOR_RUN)' \
  crates tests bin --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'
```

The first search must be empty in production. The construction/parsing and semantic-domain searches
are review gates: each production schema/reference hit must be owner-derived `PersistedSchema`, a
typed semantic identity deliberately not claiming retained bytes, stored untrusted input being
parsed, or an explicitly qualified opaque-external reference from the final disposition table. Each
domain literal must map to the reviewed typed owner operation; no callsite may select it dynamically.
Test-only arbitrary hostile IDs stay behind test support and may not be imported by production.

The moved-limit search must show imports only from the owner named in the redistribution table; no
central cross-domain limits namespace may remain.

### Backend semantic-authority review

```bash
set -euo pipefail

review_rg() {
  local review_exit=0
  rg "$@" || review_exit=$?
  test "$review_exit" -le 1
}

expect_no_matches() {
  local rg_exit
  if rg "$@"; then
    return 1
  else
    rg_exit=$?
    test "$rg_exit" -eq 1
  fi
}

expect_no_matches -U -n \
  '\b(StructuredHistoryBackend|ConfigurationHistoryBackend)\s+as\s+[A-Za-z_][A-Za-z0-9_]*' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

backend_authority_rg_exit=0
backend_authority_output=$(
  rg -l '\b(StructuredHistoryBackend|ConfigurationHistoryBackend)\b' \
    . --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'
) || backend_authority_rg_exit=$?
test "$backend_authority_rg_exit" -eq 0
mapfile -t backend_authority_files <<<"$backend_authority_output"
test "${#backend_authority_files[@]}" -gt 0

declare -A backend_review_root_set=()
for backend_authority_file in "${backend_authority_files[@]}"; do
  backend_review_root=$(dirname "$backend_authority_file")
  while test "$backend_review_root" != . && \
    test ! -f "$backend_review_root/Cargo.toml"
  do
    backend_review_root=$(dirname "$backend_review_root")
  done
  test -f "$backend_review_root/Cargo.toml"
  backend_review_root_set["$backend_review_root"]=1
done

backend_review_roots_output=$(
  printf '%s\n' "${!backend_review_root_set[@]}" | sort
)
mapfile -t backend_review_roots <<<"$backend_review_roots_output"
test "${#backend_review_roots[@]}" -gt 0

review_rg -n \
  'RunRecord|StateLeaf|StructuredFrontier|AccessKind|ObservationOutcome|TenantFactCoordinate|FactPublication|entry_budget|reassert|settlement' \
  "${backend_review_roots[@]}" --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

review_rg -n -i \
  'jsonb|json_extract|->>|#>>|#>|row_number|lead\(|lag\(|observed_outcome|access_kind|run_closed|fact_publication|configuration.*(head|revision)' \
  crates/storages/postgres/src crates/storages/postgres/migrations \
  --hidden --glob '!.git/**' --glob '!target/**'

review_rg -n \
  'fn (open_structured_authoritative|open_structured_authoritative_application|open_structured_authoritative_with_configuration|open_configuration_maintenance|assemble_structured_runtime)' \
  crates/storages/postgres crates/kernel/store --hidden --glob '!.git/**' --glob '!target/**' \
  --glob '*.rs'

review_rg -n \
  'QualificationSession|REPEATABLE READ|repeatable_read|verify_configuration_history|fence_generation|release_epoch' \
  crates/storages/postgres crates/kernel/store --hidden --glob '!.git/**' --glob '!target/**' \
  --glob '*.rs'
```

Backend traits may not be renamed through an aliased import or aliased re-export, and no macro may
generate an implementation: every `impl` must be explicit and name the exact trait. The
`backend_authority_files` array therefore enumerates every Rust source file in the whole working tree
that can define, implement, bound, wrap, or directly use either backend trait, including memory,
PostgreSQL, binaries, integration packages, configuration, corrupt/injecting test fixtures, and any
new workspace module. Review the complete trait-name census. The script then expands every hit to
its owning Cargo package and runs the semantic-token review over all Rust source in those packages,
so an implementation cannot hide a decision in a delegated helper module or a newly added backend
crate. Compare both the authority-file and package-root sets with the prior reviewed census in the
commit-3 message; every added or removed path requires an explicit ownership explanation. Allowed
hits are physical row decoding/types, mechanical command variants, comments that state the
prohibition, or test fixture assembly outside DML decisions.
PostgreSQL may store opaque canonical JSON, but no JSON operator/window/predicate may derive run
attention, attempt state, fact semantics, or configuration semantics. Do not encode the reviewed
path list as permanent lint configuration unless a stable syntax-aware lint is straightforward.
Review every opener/assembly hit as well: each production function must be async, consume the opaque
bundle once, perform its required run/configuration qualification, and return no split capability
before the final fresh target/fence check. The session/token search must lead to one snapshot-scoped
run memo and the one shared configuration verifier, not independent per-source folds or opener-local
semantic variants.

### One owner and one path: positive review

```bash
set -euo pipefail

require_matches() {
  local rg_exit=0
  rg "$@" || rg_exit=$?
  test "$rg_exit" -eq 0
}

require_exactly_one() {
  local matches
  local rg_exit=0
  matches=$(rg "$@") || rg_exit=$?
  test "$rg_exit" -eq 0
  test "$(printf '%s\n' "$matches" | wc -l)" -eq 1
  printf '%s\n' "$matches"
}

require_matches -n '\bderive_run_id\b' crates tests bin
for fact_hash_owner in \
  derive_fact_content_identity \
  derive_fact_logical_identity \
  derive_fact_query_digest
do
  require_matches -n "\b${fact_hash_owner}\b" crates tests bin
done

require_exactly_one -n \
  '^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?fn reduce_event\b' \
  crates/kernel/store/src/structured --glob '*.rs'
for reduction_type in PendingSemanticStep ComparedReduction FinalizedReduction; do
  require_exactly_one -n "\bstruct ${reduction_type}\b" \
    crates/kernel/store/src/structured --glob '*.rs'
done

require_matches -n '\breduce_event\(' crates/kernel/store crates/kernel/replay
for stable_append_token in lookup_append_attempt append_request_id; do
  require_matches -n "\b${stable_append_token}\b" \
    crates/kernel/store crates/storages/postgres
done
for validated_append_type in ValidatedRunAppend ValidatedConfigurationAppend; do
  require_matches -n "\b${validated_append_type}\b" crates
done
for assembly_authority_token in \
  QualifiedProgramRegistry \
  into_runtime_parts \
  CertifiedProcessRegistry \
  RuntimeAssemblyToken
do
  require_matches -n "\b${assembly_authority_token}\b" \
    crates/kernel/store crates/storages/postgres crates/app
done
for attention_token in has_effect_entry_attention EffectEntryAttention; do
  require_matches -n "\b${attention_token}\b" crates tests bin docs
done
```

These searches locate declared owners/callers; differently named duplicate formulas or reducers can
escape them. Inspect the hits, then use fixed/current goldens, one-field hostile mutations,
full/incremental equivalence, and construction-boundary compile tests as the actual proof that:

- callers converge on one hash owner rather than duplicate formulas;
- full and incremental orchestration call the same reducer rule;
- the three consuming typestates each have one private definition and no bypass constructor;
- stable append-request resolution happens before current frontier-dependent authoring;
- validated append construction exists only in the store coordinator/compiler;
- production assembly consumes one complete program registry and preserves the matched process/token
  authority until semantic open succeeds;
- backend use of attention is limited to the supplied boolean/current index;
- complete attention subject/resolution exists only in reducer/evidence/public projection owners.

### Diff-scope gates

The old empty `crates/kernel/journal/` gate is invalid: journal owns the shared run identity,
prior-run retained contracts, and record cutover. The old empty EVM-PostgreSQL gate is also invalid:
limit ownership and deletion of false wallet-evidence references legitimately change that crate.

Use these narrower checks instead:

- commits 4–7 must not modify the reducer's already-pinned minimum-lineage-head or
  previous-physical-binding rule except for the explicit `Reassertable` subject field; inspect their
  focused diff and retain behavior regressions;
- commit 8's EVM absorption behavior must not modify `crates/storages/evm-postgres/` relative to
  commit 7; all legitimate EVM-PostgreSQL schema/limit identity work belongs in commit 2;
- physical verification orchestration moves in commit 3, but its policy changes only to the explicit
  closed rule: retained first, then current for a new candidate, and retained only for replay;
- `git diff --check` is clean at every commit and on the final tree.

---

## Verification schedule

Use the default Nix development shell for every direct Cargo/Rust command. The commit sections name
the minimum focused evidence; expand only when an affected dependency boundary or a failure requires
it. Record exact commands/results in the commit or handoff notes.

After commit 2 changes `nixfied.nix`, run `nix run .#model-check` early and exercise both the renamed
`structured-history-postgres-qualification` leaf and the retained SQL inventory leaf. After manifest
or dependency changes, run `nix run .#run -- --task cargo-metadata-contract`. After PostgreSQL query
or metadata changes, exercise the focused SQLx and structured-history leaves. After EVM PostgreSQL
identity/limit changes, exercise its focused storage/qualification leaf selected by
`docs/build-and-verification.md`.

Before final CI:

1. complete all owner-focused tests and deletion/review gates;
2. run `nix develop -c cargo fmt --all -- --check` and `git diff --check`;
3. run the renamed structured-history and EVM PostgreSQL leaves if they have not already been run on
   the final relevant code;
4. delete this plan and rerun the no-plan deletion searches;
5. run `nix run .#ci` exactly once on the final revision.

Do not run `nix run .#check`, `.#test`, and `.#test-db` immediately before `.#ci` on the same tree;
CI already composes them. Use a component gate only as the smallest independent diagnostic if a
focused target exposes a failure.

---

## Final acceptance checklist

The engineer may delete this plan only when every statement below is evidenced:

- *Material uncertainties* says `none`, backed at the planning-anchor commit by all three reviewed
  historical-to-final proof tables. `docs/persisted-public-surfaces.md` contains their current-only
  result—retained-schema/predicate owners, bounded-shape vocabulary, and surviving semantic-domain
  owner/preimages—while the deleted-name disposition remains only in Git history.
- There is one Rust-owned persisted-shape mechanism, one direct fact request, one shared run-ID
  function, and no annex/generator/registry replacement.
- Every real retained/published byte surface has an honest owner descriptor; every codec-only,
  preimage, primitive, nested, or dead schema badge is gone.
- Portable v3 has one complete-stream `ContentRef`, a closed JSON-lines codec, and no per-frame or
  derivative chain/count identity; `PublishedEntryPoint` is output metadata with no phantom retained
  capability.
- `RunAdmitted.certified_program_ref` is the sole exact retained certified-root reference; duplicate
  root/policy/audit fields and special digests are gone.
- EVM provider v4 and wallet PostgreSQL v2 compare real retained inputs/keys/results/proofs directly;
  false evidence references and their duplicate fields are gone without a replacement digest.
- The recoverability tree, module, limits, tests, docs, Nix leaf, error/types/helpers, and every
  textual reference are deleted; valuable behavior remains under owner-local tests.
- The concrete certification registry is the only source of `CertifiedProgram`; no
  `QualifiedProgram`, qualifier/verifier trait, or store-local program copy exists.
- `VerifiedStructuredRun` pairs immutable `QualifiedHistory` with compact `ReducedRunState`; raw
  histories become qualified recorded events/assertions, its qualifier-built context inseparably
  ties the admitted program to `CertifiedProgram`, and no retained container enters reduced state.
- `reduce_event` is the sole deterministic synchronous transition rule for admission and later
  events, with no callback, verifier, IO, authoring, proposal, persistence, or generate/validate
  mode and no separate start/prepare rule.
- Candidate intent preview, requalified recorded candidate, retained full replay, and incremental
  advancement execute that same rule and satisfy both plan-equality and complete prefix-equivalence
  matrices. The coordinate-free `PendingSemanticStep -> ComparedReduction -> FinalizedReduction`
  consuming typestate is the only route to a bound successor; no preview result can be sealed.
- Physical/currentness and prior-run completeness are explicit closed obligations; portable and
  live fact verification use the same selector semantics; cacheable candidates pass retained then
  current verification, replay uses retained only, and partial/current-only discharge is impossible.
- One compiler owns deterministic records/artifacts/assignment and exact retained assertion
  comparison, distinguishes observed versus retained cross-run coordinates without reducer modes,
  and derives every run-local coordinate independently; placeholders, generated overlays, and
  `fold.rs` are gone.
- Effect attention is reducer-owned and includes actionable closable/reassertable states as well as
  terminal manual states; Read never enters it.
- Backends accept only sealed validated append commands and mechanically compare/apply supplied
  projections. They contain no independent Runtime/configuration/fact/attention reducer.
- No projection receipt or historical derived after-image exists. Exact batch identity plus atomic
  co-commit is the retry/acknowledgement proof.
- Stable `(run_id, append_request_id)` lookup resolves retained attempts before current
  head/frontier-dependent reduction; fact retries cannot be re-authored at a later tenant frontier.
- PostgreSQL v7 has no attempt table and exactly one current per-run attention partial index; memory
  and PostgreSQL expose the same current projection contract.
- Store open consumes the complete program registry and backend once, retains the admission/process/
  assembly-token coupling, uses pinned run snapshot `S0` and one exact-prefix
  `Visiting | Verified` memo, verifies configuration through its shared path, and freshly revalidates
  database/fence/release identity before capabilities escape. It compares projections in both
  directions at `S0`; sealed same-release appends preserve them inductively, and normal open never
  repairs corruption.
- `ObservationQualification`, observation/supersession preflights, `commit_prepared`, and their
  duplicate branches are gone; all Runtime mutations use the one `commit_event` coordinator.
- Attention discovery is tenant-bound, route-not-answer, live/keyset-paged by run, duplicate-free in
  a quiescent sweep, independent of resolved lifetime history, and explicit about sweep latency.
- Old parked public names/cursors/routes and all compatibility aliases/decoders are absent; CLI,
  REST, replay, and docs describe only the current contract.
- `IMPL_PLAN_EFFECT_ENTRY_RES.md` and the superseded recoverability plan/document are absent; Git
  history/private backup is the only archive.
- EVM preserves unknown-on-absence, uses the kernel closure discriminator, reasserts only within
  budget, and states the remaining two-process test seam honestly.
- All focused evidence, deletion gates, Cargo metadata/model/task checks, database qualification,
  and the one final `nix run .#ci` pass on the final tree.
- The rewritten branch contains the ordered coherent commits above, is still unpushed, and the
  private backup ref is the only local preservation of the superseded history.

Once this checklist is complete, delete this file. The resulting repository, not a retained plan or
compatibility note, is the sole current design.
