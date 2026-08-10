# TT1 solution plan: complete the Runtime-store cutover after `10c53f2a`

Status: execution handoff. Keep this file until every final acceptance item is evidenced, then delete
it in the final implementation commit and rerun the deletion gates without excluding it.

Audited baseline: `10c53f2aaad65661f9ce8885a4608cfb792893b0`.

Observed post-audit starting point when this handoff was written: `a9ad9442f`.

Frozen local remote-tracking ref at handoff: `origin/refact-runtime` =
`35fd0144a6daf9c06e33087751b1880aced87e9b`.

This plan supersedes the completion claim for the deleted
`IMPL_PLAN_FIX_RUNTIME_STORE_THREE_LAYER.md`. It does not supersede `docs/design.md` or
`docs/architecture.md`; those documents already describe the intended three-layer design. The work
below makes the implementation and evidence match those contracts.

## Material uncertainties

None.

Two apparent ambiguities were resolved before writing this handoff:

- Retain `CertifiedProgramComponents::qualified_entry_point_admission_policy_ref`. It is the sole
  certified-root edge to the qualified admission-policy component. The deleted duplicate was the
  former `RunAdmitted` field. Narrow the old global deletion gate to this owner boundary instead of
  renaming or deleting the root field.
- A configuration revision must have one content identity. `ConfigurationRevision` is the
  self-reference-free persisted payload. An opaque `ConfigurationRevisionObject` pairs the exact
  typed `HistoryObject` with its decoded payload and delegates identity to
  `HistoryObject.content_ref`; it stores no second digest. The store head and admission material use
  that same object and exact bytes. `ConfigurationHistoryHead` names that value `object_ref`; the
  deleted `revision_ref` name is not retained for either the payload or head API.

## Starting-point discipline

The branch was unpushed at the audited revision. Preserve that property until final acceptance.
Keep `pre-squash-backup` or an equivalent private backup ref while rewriting the local history.

The exact final-history parent is
`56c260ba492ef5de71e3ef82250295cd583a2868` (`d2d46752^`), the shared pre-cutover commit already
above `origin/refact-runtime`. `a9ad9442f` is the audited comparison/source tree, not an ancestor
requirement for the final branch. The final local history is exactly:

```text
56c260ba require deleting every superseded construct in the commit that orphans it
  -> plan the 10c53f2a runtime-store correction
  -> finish rust-owned persisted identity ownership
  -> replace runtime storage with three qualified layers
  -> prove evm effect attention absorption
```

It is acceptable to execute and verify the deletion-first working cutover atop `a9ad9442f` first.
After the final working tree passes focused gates, rebuild the branch from `56c260ba` into the four
commits above, harvesting the final behavior rather than the obsolete implementations. Before
deleting temporary working branches, prove the rebuilt final tree is byte-identical to the verified
working tree. The private backup ref is the only retained pointer to the superseded local commit
history. Do not merge, cherry-pick wholesale, or otherwise make `699bb63d..a9ad9442f` ancestors of
the final branch.

The shared branch advanced during the audit with two partial repairs:

- `0a09876c3 regenerate the postgres catalog attestations for the current baselines` commits the
  PostgreSQL v7 and EVM wallet PostgreSQL v2 catalog hashes that were missing from `10c53f2a`, plus
  maintenance diagnostics.
- `a9ad9442f pin the portable export version assertion to its owner` corrects stale portable-version
  assertions and documentation.

Do not blindly discard or redo those changes. If the engineer rebuilds the local commit history
from `10c53f2a`, re-harvest their behavior into the appropriate logical commits. If later schema or
portable changes alter either contract, regenerate the affected evidence again from the final
bytes. Remove temporary hash-emission diagnostics after the final manifests are pinned.

Before implementation, record:

```bash
git status --short
git rev-parse HEAD
git log --oneline --decorate -8
git rev-parse pre-squash-backup
git rev-parse origin/refact-runtime
```

The handoff file itself is expected to be the only new path at transfer time. Review it, then commit
it alone as the temporary working planning anchor `plan the 10c53f2a runtime-store correction` atop
`a9ad9442f`; that commit does not have final ancestry. Recreate the same content/subject as the first
commit above `56c260ba` during the final history rebuild. Only
after the working anchor exists must `git status --short` be empty. Do not overwrite concurrent work
or begin implementation from a tree whose ownership is unknown.

## Completion verdict for `10c53f2a`

`10c53f2a` is not the completed design. It fixed several surrounding problems, but the central
Runtime/store architecture is mostly a file split and vocabulary change around the old fold.

The following pieces are real and must be retained:

- immutable append-only run batches and objects remain the durable semantic authority;
- `run_access_attempts` is gone;
- PostgreSQL has one per-run current head row and one partial
  `(tenant_scope_id, run_id) WHERE has_effect_entry_attention` index;
- historical attempts no longer grow the attention route or cause static cross-page duplicates;
- the keyset/sentinel pagination algorithm advances by the last scanned run and has the intended
  sparse-page live-sweep behavior;
- route-not-answer filtering prevents a forged positive route from becoming false semantic
  evidence;
- the recoverability annex/generator/task plane is deleted;
- `RunAdmitted.certified_program_ref` is the sole admission-level certified-root reference;
- the false EVM wallet-storage evidence references are deleted and provider/storage versions are
  advanced;
- EVM preserves `Ok(None) | Err(_) => EntryUnknown`; absence never becomes a non-entry claim;
- the Effect entry-mode trait remains sealed and the minimum-lineage-head and previous-physical-
  binding rules remain strict.

Passing broad Cargo tests does not compensate for the missing ownership boundaries below. The
deleted plan required source/deletion gates, live database qualification, corruption regressions,
and one final `nix run .#ci` on the exact final revision.

## Deletion-first correction mandate

This is not an incremental cleanup of the code added by the recent commits. Several new files have
the desired architectural names while retaining the old implementation and its mixed
responsibilities. Their names are not evidence and their current bodies are not scaffolding to
preserve. Retain proven semantic behavior; delete the implementation that places it under the wrong
owner and rebuild the smallest current path.

The correction baseline is `a9ad9442f`. Relative to `d2d46752`, that tree changes 180 files with
16,013 insertions and 21,340 deletions. More importantly, `dfd92bcb` alone added 7,393 lines while
claiming the three-layer cutover:

| Addition in `dfd92bcb` | Added lines | Required disposition |
| --- | ---: | --- |
| `structured/reducer.rs` | 2,762 | Delete the current implementation and rebuild the pure reducer |
| `structured/qualification.rs` | 1,171 new / 21 removed | Delete the mixed-fold half; rebuild the qualified-history owner |
| `structured/coordinator.rs` | 1,037 | Delete the current implementation and replace parallel event pipelines with one coordinator |
| `structured/test_support.rs` | 779 new / 453 removed | Delete bypasses and copied semantic formulas; retain only fixtures that enter through real owners |
| `structured/compiler.rs` | 305 | Delete the current generic factory implementation and rebuild the one typed compiler |
| `structured/obligations.rs` | 181 | Delete the free partial-check implementation and rebuild one closed consuming coordinator |
| `structured/projection.rs` | 117 | Delete whole file; no projection compatibility module remains |
| `structured_history_qualification.rs` | 230 | Replace the weak suite; do not retain names-only or no-comparison tests |

The old 5,042-line `fold.rs` and 5,409-line inline test module remain deleted. Do not restore either
file and do not transplant their containers under new names. Port only exhaustive semantic rules and
regressions into their final owners.

Before replacements are written, gross-delete the current bodies of `reducer.rs` (2,762),
`canonical_append.rs` (474), `mutation.rs` (497), `assembly.rs` (80), `coordinator.rs` (1,037), and
`obligations.rs` (181): 5,031 physical lines of the wrong implementation. This is a deletion-first
working checkpoint, not a commit—the tree may be temporarily uncompilable. The final logical commit
must still be coherent. Record the checkpoint diff, then add only the smaller final owners. Do not
count moves, formatting, test deletions, or plan deletion toward this 5,031-line gross-removal floor.

At that checkpoint, run the following against the untouched correction-baseline parent and preserve
its output in the engineer completion report and commit-2 body. It must report zero additions and at
least 5,031 deletions; do not commit the broken checkpoint:

```bash
set -euo pipefail
delete_checkpoint_paths=(
  crates/kernel/store/src/structured/reducer.rs
  crates/kernel/store/src/structured/canonical_append.rs
  crates/kernel/store/src/structured/mutation.rs
  crates/kernel/store/src/structured/assembly.rs
  crates/kernel/store/src/structured/coordinator.rs
  crates/kernel/store/src/structured/obligations.rs
)

git diff HEAD --numstat -- "${delete_checkpoint_paths[@]}" |
  tee /dev/stderr |
  awk '
    $1 ~ /^[0-9]+$/ && $2 ~ /^[0-9]+$/ { added += $1; deleted += $2 }
    END {
      printf "delete-first checkpoint: additions=%d deletions=%d\n", added, deleted
      if (added != 0 || deleted < 5031) exit 1
    }
  '
```

### Disposition vocabulary

The engineer must apply these meanings literally:

- **Delete whole file**: remove the path and every module declaration, import, fixture, generated
  artifact, and documentation reference. Do not leave a facade.
- **Delete current implementation and rebuild**: preserve the module name only for the explicit
  recreate rows (`reducer.rs`, `compiler.rs`, `coordinator.rs`, `obligations.rs`, and the retained
  half of `qualification.rs`). Remove every current type/function listed below before constructing
  the replacement. Copying it to another module or renaming it does not satisfy deletion.
- **Delete symbol/family**: remove the definition, all calls, branches, fields, serialized names,
  SQL columns/constraints, tests, `.stderr`, docs, and generated SQLx metadata for that concept.
- **Retain but rewrite**: preserve the one valid responsibility or behavior, not the existing
  representation or control flow.

No compatibility alias, deprecated wrapper, forwarding method, alternate constructor, legacy
decoder, old JSON field, old SQL column, receipt, historical after-image, or comment describing a
superseded current path may remain. Git history and this planning-anchor commit are the archive.

### Disposition of files added in the implementation range

Every file added by `d2d46752..a9ad9442f` has an explicit disposition:

| Added path/family | Disposition |
| --- | --- |
| `crates/kernel/store/src/structured/{reducer,compiler,coordinator,obligations}.rs` | Delete each current body and rebuild only the final owner described below |
| `crates/kernel/store/src/structured/projection.rs` | Delete whole file; compiler owns projection compilation and `validated_append.rs` owns data-only row/plan types |
| `crates/kernel/store/src/structured/mutation.rs` | Delete whole file; the one coordinator replaces it |
| `crates/kernel/store/src/structured/assembly.rs` | Delete whole file; replace it with `semantic_open.rs`, not a facade |
| `crates/kernel/store/tests/structured_history_qualification.rs` | Replace the current suite; the existing prefix test proves no full/incremental equality |
| `crates/kernel/store/tests/ui/fail/program_qualifier_is_not_a_public_trait.{rs,stderr}` | Delete whole files; failure on an invented import proves no real boundary |
| `crates/kernel/store/tests/ui/fail/validated_append_cannot_be_counterfeited.{rs,stderr}` | Replace the entire fixture so it attacks the consuming proof seam, not only field privacy |
| `crates/kernel/store/tests/ui/fail/program_trust_requires_the_certification_registry.{rs,stderr}` | Rewrite both files against the final concrete certification/semantic-open boundary; the proof remains mandatory |
| `crates/kernel/canonical/src/limits.rs` | Retain the owner constants, wire them into hostile ingress, and delete downstream substitute checks |
| `crates/kernel/values/src/persisted.rs` and its persisted-contract tests | Retain the bounded descriptor algebra; delete string-selected use of it for known Rust values |
| program-derive persisted-schema tests | Retain and extend for concrete closed owner shapes |
| Effect entry-mode positive/compile-fail tests | Retain the sealed-mode behavior; update only final public owner paths |

No other newly added file, helper, or branch is implicitly protected. In a mixed-purpose file,
retaining one valid owner does not authorize keeping adjacent superseded factories, wrappers, or
alternate paths. Delete every tombstoned family below from that file and record every other
production symbol introduced by `d2d46752..a9ad9442f` in the final same-name/rehosted-code review as
either part of an explicitly retained owner or deleted. “The file is still needed” is never a
disposition for code inside it.

### Already-deleted planes that must stay deleted

Do not resurrect deletions that were correct merely because the final history is rebuilt from the
pre-cutover parent. The final tree contains none of:

- `contracts/recoverability/`;
- `crates/kernel/canonical/src/recoverability.rs` or `recoverability_limits.rs`;
- `crates/kernel/canonical/tests/recoverability_v1.rs`;
- `docs/recoverability-app-surface-v1.md`, `docs/recoverability-predicate-owners-v1.md`, or the
  working design note `docs/recoverability-removal.md`;
- the `recoverability-postgres-v1` Nix task;
- generated annex/corpus/hex artifacts, `RecoverabilityContract`, `RecoverabilityErrorCode`, or
  `CanonicalReferencePath`.

The bounded constants now owned by `mfm-canonical::limits` remain. This is an absence requirement,
not permission to recreate the annex under another name.

### Runtime/store tombstone ledger

#### `reducer.rs`: discard the mixed reducer body

Delete the current 2,762-line file wholesale, then recreate the path with the small pure reducer.
The following current families
must not survive under aliases:

- `FoldMachine`, `VerifiedFoldState`, `DerivedProgram`, `PhysicalObligationScope`, and every raw
  history/object/assigned-record field they retain;
- `derive_program(&FoldMachine)`, the machine-bound `DerivationEngine`, and any traversal that
  reaches into retained objects or the certification closure during each event;
- reducer-side `HistoryObject`, `CommittedBatch`, `AssignedRecord`, `RunRecord`, `serde_json`,
  canonical serialization, framework-object construction, candidate assignment, and placeholder
  hash dependencies;
- reducer-side physical checker parameters and calls;
- repeated `find_state*`, `decode_component`, `validate_program_value_schemas`, typed-value object
  decoding, secret-marker scans, and resource-lineage/component-closure walks already owned by
  qualification/certification;
- the current dead `QualifiedEvent::Intent(CommittedBatch)` representation;
- the current `ReducerState`, `PendingSemanticStep`, `ComparedReduction`, and
  `FinalizedReduction` wrappers around the same `DerivedProgram`;
- the current `compare_recorded`, which compares a batch with values borrowed from itself, and the
  unconditional `discharge`;
- broad impossible-combination fallback to `BlockedIntegrity`.

Re-express the necessary workflow rules as the one free `reduce_event(context, previous, event)`
over already qualified typed values and compact state. Do not preserve any helper from the current
file merely because its local calculation is useful. After the wholesale deletion, recreate a
helper only from the final reducer-owned semantic inputs and outputs, with no callback, raw
evidence, serialization, authoring, obligation, or persistence dependency; the deletion checkpoint
must prove that no old helper body was carried through in place.

#### `coordinator.rs`: discard every parallel event pipeline

Delete the current 1,037-line file wholesale, then recreate only the small coordinator, including
removal of:

- `prepare_validated_genesis` and `extend_verified_candidate`;
- `reduce_admission_event`, `reduce_transition_event`, `reduce_authorization_event`, and
  `reduce_observation_event` as separately authored semantic paths;
- `validate_transition_fact_coordinate`, `authorization_requires_fact_selection_barrier`, and
  `validate_authorization_fact_coordinate` as coordinator-owned semantic decisions;
- `qualify_observation_event` and `proposed_observation` preflight logic;
- `assign_candidate` and its coordinator-owned record assignment;
- `AssignedRecordHashPreimage`, `AssignedCommitDigestPreimage`, `BorrowedCommitCandidate`, and any
  local duplicate of journal/compiler identity ownership;
- the `mfm.structured-placeholder-access-attempt.v1` sentinel and every placeholder coordinate.

Replace this body with one small `commit_event` orchestration path: stable lookup, intent reduction,
frontier acquisition where required, compile, owner-qualify exact bytes, recorded reduction,
comparison, whole obligation discharge, sealed append, mechanical backend call, and result
classification. The coordinator decides ordering; it does not repeat event semantics or codecs.

#### `qualification.rs`: remove the fold and output-state half

Delete:

- `VerifiedFoldState`, `fold_recorded_history`, and the current coupled
  `verify_recorded_history` implementation;
- `ReducedEvent`, `ReducedObservationEvent`, `RecordedTransition`, `RecordedAuthorization`,
  `RecordedObservation`, and `DerivedProgram` as mixed qualification/semantic products;
- `HistoryObjectLookup`, `HistoryObjectOverlay`, generated-object overlay paths, and raw lookup
  callbacks entering the reducer;
- repeated `certified_component`, `decode_component`, and `validate_program_value_schemas` scans;
- frontier, `StateLeaf`, cursor, attention, terminal-state, or actionable-state derivation;
- the current `verified_runs_equivalent` helper unconditionally; equivalence belongs in direct
  equality/regression assertions over the complete private qualified/reduced products, not a public
  or test-support facade;
- `ObligationDischargeScope` from this module; obligation scope belongs to the closed coordinator.

Retain and tighten only hostile raw-envelope qualification, exact owner decode, certified-program
memoization, batch/object/reference integrity, admitted-material checks, and construction of
`QualifiedRunContext`, `QualifiedHistory`, `QualifiedBatch`, and `RecordedAssertions`.
`VerifiedStructuredRun` itself is rewritten to exactly `{ history, reduced }`.

#### `compiler.rs`: remove generic factories and rebuild one compiler

Delete the complete current body before rebuilding, including
`state_outcome_schema_id`, `operation_outcome_schema_id`, `fact_claim_schema_id`,
`framework_object`, generic `proposed_typed_object`, shape-erased object canonicalization, the
`framework_schemas` marker module, empty `StateOutcomeObject`/`OperationOutcomeObject`/
`FactClaimObject` schema badges, and marker-shaped `StructuredFactClaimPreimage`.
Replace them with concrete persisted owner types and one compiler that owns deterministic record
assignment, artifact construction, exact preview/recorded comparison, object-suffix comparison,
`TenantFactProjectionPlan`, and data-only `FactScanPermitSpec`.

Do not retain a generator and a validator. Candidate compilation and recorded comparison are two
uses of one deterministic compilation algorithm.

#### `obligations.rs`: remove freely callable partial checks

Delete the current 181-line file wholesale and recreate one consuming dispatcher. Retain the
behavior of prior-run selection and physical authorization/supersession checks only behind the
closed obligation set.
`PhysicalObligationChecker` and its seal remain an external capability, but no reducer or event-
specific pipeline may invoke it directly. There is no `CurrentOnly`, optional discharge, public
token, boolean, or unconditional finish.

#### Delete `projection.rs` whole

Delete every current function: `derive_frontier`, `state_frontier`, `fan_out_frontier`,
`effect_entry_subject`, and `minimum_action`. Frontier and attention come from the reducer; the
compiler copies finalized values into transient after-images. Delete the file, its module
declaration, and all imports. Put the data-only `RunCurrentProjection` and sealed
`RunProjectionPlan` next to `ValidatedRunAppend` in `validated_append.rs`; do not create another
projection module.

#### Delete `canonical_append.rs` whole

Delete `crates/kernel/store/src/structured/canonical_append.rs` and all of:

- its current bypassable `ValidatedRunAppend::new` and `ValidatedConfigurationAppend::new` seams;
- `validate_append_objects`, `validate_record_object_closure`, nested-reference collection, and
  envelope/frame validation duplicated from owner qualification;
- any accessor that extracts only a batch while dropping projection/fact commands.

Create `validated_append.rs` only for sealed read-only backend commands constructible by the one
compiler/coordinator seam from `FinalizedReduction`. Do not copy the old file and rename it.
`RunCurrentProjection` survives only as the data-only row image in `validated_append.rs`;
`RunProjectionMutation` does not survive and is replaced by the sealed `RunProjectionPlan` defined
below. No other projection type or module survives.

#### Delete `mutation.rs` and `assembly.rs` whole; shrink adapter/backend

Delete `crates/kernel/store/src/structured/mutation.rs` whole. Do not keep the path as a forwarding
facade. Its responsibilities move into the one coordinator and final proof/result owners. In
particular, do not preserve:

- `ObservationCommit::ExistingSame(VerifiedStructuredRun)` and every prepared/candidate successor
  cached for `ExistingSame`;
- `StructuredAppendAttempt.successor: Option<_>` as a value shared by newly committed and
  historical outcomes;
- precommit `fact_scan_permit(...)` construction and the stored optional live permit;
- the four event-specific reduction calls and observation preflight method;
- result conversions that release the candidate successor on both `NewlyCommitted` and
  `ExistingSame`;
- late-only candidate-digest `resolve_append` as the first idempotency lookup.

Delete from `adapter.rs` the head-only cache probe, `current_head` comparison, observation preflight
port, duplicate proposal-cloning conversions, and every `ExistingSame` branch that caches a
candidate-built successor. A retained exact retry reloads the actual current qualified run.

Delete from `backend.rs`:

- `StructuredRunSnapshot.head`;
- trait/writer `current_head`;
- redundant raw `load`/head-only entries when `load_snapshot` owns the complete snapshot;
- any append API accepting an ordinary batch/projection tuple rather than the sealed command;
- semantic classification of an append lookup result.

Add only `current_run_projection` and mechanical
`lookup_append_attempt(run_id, append_request_id)` alongside the exact snapshot/prefix reads.

Delete `crates/kernel/store/src/structured/assembly.rs` whole, including
`assemble_structured_runtime` and `assemble_with_backend`. Create
`crates/kernel/store/src/structured/semantic_open.rs` with the one async entrypoint
`qualify_and_open_structured_store`; do not keep an assembly facade or a second opener. Delete
direct pre-audit `StructuredRunStore::new(...).split()` paths and every test/production opener that
exposes a reader, writer, Runtime, or configuration capability before semantic open completes.

#### Memory and test-support deletion

Delete memory-side reconstruction of facts, configuration meaning, attention, or recovery from
`RunRecord`, `TenantFactCoordinate`, cursor, or leaf variants. Memory installs only compiler-supplied
commands and compares exact current values.

Delete from `test_support.rs` every copied run-ID/hash/object formula, permissive qualifier/checker,
direct store split, fabricated verified state, or fixture backend that bypasses semantic open. A
corrupting backend may mutate physical rows/commands for negative tests; it may not become a second
semantic implementation.

### Runtime observation and attention tombstones

Delete the observation-preflight family end to end:

- `runtime/history/cursor.rs::ObservationEventQualification`;
- its re-export from `runtime/history/mod.rs`;
- `RuntimeHistoryPort::qualify_observation_event`;
- Runtime `qualify_invoked_observation_event` and `PendingObservation`;
- store writer/adapter/coordinator `qualify_observation_event`;
- preflight `Ready`, `ExistingSame`, and `InvalidSupersessionEvidence` branches;
- the separate supersession preflight and duplicate physical checks.

Also delete the event-specific Runtime/store plumbing that exists only for that split path:

- unused public `AppendAttemptApi` and `AuthorizationApi` traits and implementations;
- `RuntimeHistoryPort::retain_verified` and adapter/store implementations;
- late `resolve_attempt`/old `resolve_append(run_id, append_request_id, candidate_digest)` APIs;
- `runtime/history/proofs.rs::StructuredAppendAttempt.candidate`, its candidate-digest accessor,
  and mutating `confirm_existing_same`;
- both Runtime and store `ObservationCommit` enums;
- cosmetic `CommittedObservation<K>`, `ObservationRetryBackoff`, and every construct-and-drop site;
- Runtime's late retry/reauthoring loops for transition, authorization, and observation;
- store public test facades `assign_candidate_for_test` and `verified_runs_equivalent`.

Replace event-specific mutation methods with one sealed commit operation over normalized
`QualifiedRuntimeIntent`. Stable lookup occurs inside the store before tenant reads or authoring;
Runtime does not retain/correct/cache a candidate successor itself.

The sole live-permit constructor is named `mint_committed_fact_read_capability` and is private to
`coordinator.rs`. It combines `FactScanPermitSpec` with live dependencies only inside the exact
`NewlyCommitted` arm. Delete the old `fact_scan_permit` function/field vocabulary completely.

For attention, retain the one per-run route, partial index, keyset cursor, grant, CLI command, and
REST endpoint, but delete the incomplete manual-only interpretation added by `57ba0107`:

- delete `VerifiedStructuredRun::current_projection` logic using
  `matches!(frontier, PossibleEntry(_))`;
- delete reader filtering that accepts only `StructuredFrontier::PossibleEntry`;
- delete manual-only rustdocs/comments;
- replace `EffectEntryAttentionEvidence`/`EffectEntryAttentionEntry` bodies that carry only
  occurrence and attempt with the full reducer-owned subject plus resolution;
- change `StateLeaf::Reassertable` so the immediately preceding attempt ID is retained;
- delete any public/private second derivation of `Manual | CloseThenReassert | Reassert`.

The sole reducer product is conceptually:

```rust
struct EffectEntryAttention {
    subject: EffectEntrySubject,
    resolution: EffectEntryAttentionResolution,
}

enum EffectEntryAttentionResolution {
    Manual,
    CloseThenReassert,
    Reassert,
}
```

Place the closed `EffectEntryAttentionResolution` enum next to the other typed cursor/leaf domain
values in `runtime/history/cursor.rs`; it contains no derivation logic. Define the sole
`EffectEntryAttention` product in `store/structured/reducer.rs`. Define the store inventory page in
`store/structured/purpose.rs`. App surface types map/read those values but do not define another
resolution enum.

Delete store `EffectEntryAttentionEvidence`/`EffectEntryAttentionPageEvidence`; the store reader
returns `EffectEntryAttentionPage` containing the reducer-owned attention values and continuation
key. The app's public `EffectEntryAttentionEntry` is a redaction-safe mapping of the same subject
plus resolution, never a second classifier.

The attention endpoint, reader, cursor, and page names introduced by commit 7 remain the one current
inventory API. Rewrite their payload and authority; do not add a second “actionable attention”
endpoint. This retention does not apply to the incorrectly scoped `RunAccess*` policy names below.

Delete the duplicate cursor-version authority in `app/surface.rs`: keep one versioned prefix, remove
`EFFECT_ENTRY_ATTENTION_CURSOR_VERSION`, remove the wire `version` field and encode/decode body
comparison, and continue rejecting unknown prefixes. Delete historical “old parked/attempt cursor”
wording.

Delete `serde::Serialize` derives and Serde field attributes from
`EffectEntryAttentionEntry`/`EffectEntryAttentionPage`. `app/render.rs` is the one reviewed public
JSON encoder; a derived serializer is a second output contract. Make every field private. Provide
only read-only `run_id()`, `journal_head()`, `subject()`, and `resolution()` accessors on the entry,
and `entries()`/`next_cursor()` on the page. There are no public fields, mutable accessors, generic
serialization traits, or constructors outside the app's authoritative mapping path.

Commit 7 also put tenant-wide inventory authority inside types named `RunAccessGrant` and
`RunAccessPolicy`. Since compatibility is irrelevant, replace those public names with the one exact
family `ApplicationAccessGrant` and `ApplicationAccessPolicy`. The target remains one closed policy
concept over `AccessTarget::{AdmitTarget, RunTarget, TenantTarget}`. Delete the old types, imports,
implementations, docs, and fixtures without aliases. A private run-specific typestate may remain for
narrowing an already authorized `RunTarget`; it is not a second policy.

### PostgreSQL and configuration tombstone ledger

#### Delete semantic SQL and redundant fact summaries

From `0001_store.sql`, PostgreSQL Rust, tests, catalog manifests, and SQLx metadata, delete:

- `tenant_fact_heads.publication_count`, `minimum_order`, and `maximum_order`;
- all constraints, parsers, row structs, SELECT/INSERT/UPDATE bindings, aggregate probes, comments,
  and fixtures for those columns;
- persistent zero-head inserts; zero is absence of a head row;
- `PendingTenantFactPublication`, `TenantFactRouteSummary`,
  `TenantFactRouteSummary::is_dense_through`, and equivalent memory structs;
- backend branching on `TenantFactCoordinate::*`, `RunRecord::StateTransitionCommitted`, fact
  emptiness, or any journal/Runtime enum to manufacture a projection command;
- JSON-path fact-publication reconstruction and asymmetric route-only qualification.

Delete the call and the entire `schema.rs::validate_prefix_integrity` function—currently roughly 199
lines—including `ordered_configuration`, `invalid_configuration`, all `lag`/`row_number` windows,
fact-summary aggregation, and JSON-path semantic interpretation. Catalog-manifest JSON remains
physical attestation; retained batch adjacency and every data-dependent history/projection check
move to the pinned Rust `S0` qualifier.

Keep `tenant_fact_publications` as a disposable route, a minimal positive `tenant_fact_heads` row,
the one current `run_history_heads` row, and the partial attention index. Add the exact
`(run_id, head_sequence)` foreign key to the immutable batch. These structures are applied from a
sealed plan, never inferred by SQL.

#### Delete duplicate configuration representation

From `store/structured/configuration.rs`, delete `RevisionPreimage`, serialized
`ConfigurationRevision.revision_ref`, its descriptor/accessor/formula, independently supplied
expected/successor references, and every comparison between the bespoke reference and the later
`HistoryObject` reference.

From `app/production_structured.rs`, delete manual `canonical_json(configured.revision())`,
`HistoryObject::new`, schema selection, and app-side construction of the configuration admission
object. Reuse the exact qualified `ConfigurationRevisionObject`.

From PostgreSQL, delete:

- `value_contract_schema_id`, `value_contract_digest`, `value_schema_id`, and `value_digest` from
  `configuration_revisions`;
- their checks, row fields, parsers, query bindings, fixtures, manifests, and SQLx metadata;
- direct `serde_json::from_str::<ConfigurationRevision>` plus bespoke-reference reconstruction;
- calls to `verify_configuration_history` from DML methods;
- configuration semantic windows in schema qualification.

Retain `revision_schema_id`/`revision_digest` only as the exact typed object's `ContentRef`, minimal
physical route/sequence/predecessor/append coordinates, exact heads, and mechanical CAS.
`ConfigurationHistoryHead { sequence, object_ref }` and `predecessor_ref` are the only head/link
vocabulary. Delete every `revision_ref` field, method, local formula, serialized key, and forwarding
alias across store, app, and PostgreSQL.

Delete memory `raw_head`, the unused `full_loads` metric, and the duplicate append-answer map.
Mechanical stable lookup derives its route from immutable batches; it does not maintain another
answer index. Change historical prefix loading from a sequence-only request that consults the
current head to an exact requested `JournalHead` snapshot read.

Delete the stale PostgreSQL README reference to `ExternalCheckpointAuthority`; that type is absent
and append acknowledgement/idempotency now belongs to the stable lookup plus mechanical append
classification contract. Do not replace it with another checkpoint-authority abstraction.

#### Delete temporary catalog emitters

After the final destructive baselines are pinned, delete the temporary work from `0a09876c3`:

- `MFM_EMIT_CATALOG_MANIFEST` diagnostic branches and `CATALOG-MANIFEST` printing;
- ignored `emit_catalog_manifest_hashes_for_the_current_baseline`;
- `catalog_manifest_hashes_for_maintenance` and exports used only by that probe.

Retain the regenerated constant hashes and live qualification tests. If permanent generation
tooling is desired later, design one reviewed Nix-owned task separately; do not leave an ignored
caller-selected-schema emitter in production crates.

### Persisted identity, fact, and portable tombstones

The persisted-identity correction is also deletion-first. The recent values/facts/journal/spec/
certification work added roughly 3,101 net lines but left its typed path unused in production. Gross
deletion across these surfaces must be at least 1,100 lines, and the persisted-identity production
slice must remain net-negative after concrete owners replace the erased machinery. Moving a factory,
manual parser, marker schema, or cached reference does not count.

#### `mfm-values` and derive cleanup

Retain the bounded persisted descriptor algebra, but delete its false/manual owner layers:

- delete `COMPONENT_OBJECT_EVIDENCE_CONTRACT_BYTES`, empty
  `ComponentObjectEvidenceContract`, and the free canonical/reference functions around it; replace
  them with one real serializable literal payload;
- change `RetainedValueContract.media_type: String` to the checked `MediaType`;
- delete `RetainedValueContract::strict_decode`, `canonical_json`, `validated`, and its custom
  `Deserialize`; implement the one ordinary typed canonical owner path;
- change both fact emission media-type strings to `MediaType` and delete their contradictory local
  ASCII/255-byte checks;
- put the sole typed canonical raw-byte `ContentRef` derivation on the persisted owner trait so
  bespoke `exact_content_ref`/manual raw-reference helpers can disappear;
- move the persisted object-kind owner trait below journal as described below.

From the `PersistedSchema` derive, delete the spawned 8 MiB OS thread used to construct schema
identity and the generated unconditional `validate() -> Ok(())`. Schema identity cannot depend on
thread creation, and validation cannot be counterfeit. Reject or require explicit bounds for
unbounded `String`, `Vec`, non-empty sequence, and `BTreeMap` persisted mappings; do not hash an open
shape while a separate constructor enforces the real bound.

#### Direct fact owners

Delete from `facts/selection.rs`:

- `fact_selection_request_schema_id`, `fact_selection_query_schema_id`, `selection_schema_id`, and
  their `OnceLock<BTreeMap<name, SchemaId>>` registry;
- `fact_query_digest_over_canonical_request`; the typed request owns its fixed-domain digest;
- `SELECTOR_CONTRACT_BYTES` and empty `PriorRunFactSelectorContract`; use one real literal payload;
- `FactSelectionQuery.canonical`, manual `from_canonical_value`/`from_parts`/`canonical_value`, and
  manual schema/reference operations;
- the outer `MfmValue` base64-wrapper schema and inner open-terminal schema duality;
- `FactSelectionRequest::from_canonical_value`, `from_parts`, manual Serde, manual
  `schema_id`/`content_ref`, and every accessor that reparses its cached document.

The surviving request directly stores admitted source manifest, closed producer scope, closed
completeness mode, scan bounds, selector reference, bounded non-empty ordered queries, and its
literal version. It has one schema and one encoding.

Delete from `facts/value.rs` the empty `FactCanonicalValue` marker, cached
`PlainCanonicalJsonBytes` in `FactScalar`/`FactSubject`/`CanonicalFactPredicate`, `parse_canonical`,
the second local `lift`, and per-wrapper manual schema/reference methods. Use one closed direct
fact-value owner. Do not delete `FactSelectionReadResponse`; its opaque response boundary is a
separate intentional dependency seam.

Delete wrapper-protecting fact fixtures, including the synthetic production-invalid request and
the request half of `fact_read_values_use_secret_marker_safe_canonical_byte_wrappers`. Rewrite
goldens/hostile cases against direct owner bytes while preserving semantic selection and bounds.

#### Journal identity owners

In addition to deleting arbitrary `HistoryObject::new`/`decode`, delete:

- forwarding `prior_run_fact_source_manifest_schema_id` and
  `prior_run_fact_scanner_binding_schema_id`;
- generic `persisted_identity(name, shape)`; each retained journal document obtains its identity
  from its concrete persisted owner rather than a caller-supplied name/shape pair;
- bespoke `to_history_object`/`from_history_object` construction that repeats object kind, schema,
  canonicalization, and decode checks; use the common typed path;
- generic `derive_candidate_digest<T>`, `derive_access_attempt_id<T>`,
  `derive_record_hash<T>`, and `derive_commit_digest<T>`;
- generic `DomainEnvelope<'a, T>` and `domain_envelope_digest<T>(domain, value)`. A retained run or
  fact identity canonicalizes its own exact named preimage under a fixed owner-local domain; no
  generic envelope type or string domain parameter remains, even privately.

Each journal identity uses one exact journal-owned preimage type. Delete duplicate “serializes the
same” preimages from store/certification: reducer `AccessAttemptPreimage`, certification
`ExpectedAccessAttemptPreimage`/`ExpectedRecordHashPreimage`, and coordinator
`AssignedRecordHashPreimage`/`AssignedCommitDigestPreimage`/`BorrowedCommitCandidate`.

Delete `TestHistoryPayload` and its trait implementations; rewrite typed-object tests using real
production owners. Arbitrary raw-envelope construction belongs only to explicit hostile test
support.

#### Spec owner cleanup

Delete `mfm_spec::exact_content_ref`, `public.rs` generic `validated`/`decode`,
`capability_expansion_requirement_schema_id`, dead
`StructuredLiveComponentContract::schema_name`, and dynamic `component_schema_id`. A known live
component has one closed owner schema with `component_kind` in its payload, not four name-selected
schema clones.

Delete bespoke raw-reference methods after the common typed owner operation exists:

- `StructuralPath::content_ref`;
- `StructuredLiveComponentContract::content_ref`;
- `SecretFreeExecutableIdentity::content_ref`;
- `SecretFreeQualificationArtifact::content_ref`;
- `SecretFreeImplementationDescriptor::content_ref`;
- `AuthoredStructuredProgram::content_ref` and `ExpandedStructuredProgram::content_ref`;
- `ExpandedLexicalSlot::content_ref` and `StructuredExpansionProfile::content_ref`;
- `CertifiedProgramRoot::content_ref`, forwarding `CertifiedProgramDocument::content_ref`, and
  `PlanningProfile::content_ref`.

Retain genuinely semantic derivations such as occurrence, fragment-boundary, and failure-plan
identities. Delete generic `policy_expansion_recipe_ref<T: Serialize>` and put the operation on the
real `PolicyExpansionRecipe` owner.

Replace open factory-owned documents—policy-proceed placeholder, capability expansion requirement,
Never/access-fault contract, authored/expanded program, lane outcome, and fan-out join—with their
actual typed owners.

Do the same for the four EVM wallet policy documents. Delete the complete
`wallet_policy_schemas` module, its generic `identity(name, fields)` builder, and its field-selected
`text`/`flag`/`count`/`text_set` helpers. Replace the `serde_json::json!` document plus separately
assembled schema pairs with four concrete private serializable persisted owner types:
`EvmWalletNoncePolicyDocument`, `EvmWalletAssurancePolicyDocument`,
`EvmDeterministicSigningProfileDocument`, and `EvmSubmissionExpansionPolicyDocument`. Each owner
fixes its own literals and bounds, produces its own canonical bytes, and derives its own
schema/reference through the common typed owner operation. Do not retain a shared name/field schema
factory or a generic wallet-policy envelope.

#### Certification owner cleanup

Delete from certification:

- `typed_content_ref`, `certification_document_schema_id`, and
  `CERTIFICATION_DOCUMENT_SCHEMAS`;
- generic `component_object<T>`, generic `decode_component<T>`, string-derived
  `contract_object_type`, and duplicate `validate_component_object`;
- private `QualifiedStructuredEntryPointPolicy.admission_policy_ref`,
  `QualifiedPolicyPreimage`, and `derived_admission_policy_ref`;
- function-local erased `StructuredEntryPointContract` and open marker strings/arrays for kernel
  qualification and predicate-set documents.

Rewrite the current string-switch `validate_qualified_closure_objects` through a closed concrete
owner registry. Object kind selects the exact owner before decode; no schema-name guessing,
trial-decoding, or re-creation of a generic envelope remains. Similarly delete
`RetainedValueContract::strict_decode` trial discovery in `qualified_value_schemas`.

Delete `fixture_component_with_same_schema`; mutate explicit raw hostile envelopes instead.
Preserve hostile substitution behavior while deleting helpers that pair arbitrary bytes with a
template schema.

Delete these production generic factories and every caller, test helper, and comment preserving
them:

- `structured_content_ref` (currently 23 hits in `mfm-spec`);
- `component_kind_schema_id` and `canonical_document_schema_id`;
- `typed_content_ref` (including certification test callers) and
  `certification_document_schema_id`;
- `selection_schema_id`;
- EVM `wallet_descriptor_ref`;
- EVM `wallet_policy_schemas::{identity,text,flag,count,text_set}` and the generic module itself;
- journal `persisted_identity(name, shape)`, `DomainEnvelope`, and
  `domain_envelope_digest(domain, value)`;
- store `framework_object`/`framework_object_ref`;
- public `domain_content_digest(domain, value)` and every production/test caller;
- EVM `hash(domain, value)` wrappers around it;
- `mfm.structured-placeholder-access-attempt.v1` and
  `mfm.portable-export.digest-placeholder`.

Each surviving hash has one named typed preimage and fixed owner operation. Delete bespoke
`CertifiedProgramRoot::content_ref` and forwarding `CertifiedProgramDocument::content_ref`; obtain
the root reference from its one retained typed object. Retain
`CertifiedProgramComponents::qualified_entry_point_admission_policy_ref` only in the exact approved
three-file owner census; it is not part of this deletion family.

Delete the exact obsolete EVM evidence family from code, persisted fields, fixtures, goldens, and
current documentation: `evidence_reference`, `mfm.evm.wallet-storage-evidence`,
`reservation_evidence_ref`, `activation_evidence_ref`, `completion_evidence_ref`,
`winning_activation_evidence_ref`, `predecessor_activation_ref`, `observed_floor_ref`, and
`original_terminal_witnesses_ref`. Do not retain a “removed fields” list in current public-surface
documentation; Git history records it.

This is deletion of a false identity, not a rename to another evidence concept. Apply this exact
replacement map in commit 1:

| Current pseudo-evidence field | Final source of truth |
| --- | --- |
| `ReservedWalletNonce.reservation_evidence_ref` | Delete; the retained `semantic_reservation_key: EvmNonceReservationKey` is the permanent row/operation identity |
| `ActiveWalletCandidate.activation_evidence_ref` | Replace with `candidate_operation_key: EvmCandidateOperationKey`, equal to the request, derived ordinal key, and SQL primary key |
| `CandidateActivationPermit::Replacement.predecessor_activation_ref` | Replace with `predecessor_candidate_operation_key: EvmCandidateOperationKey` |
| `CanonicalTerminalOutcome.winning_activation_evidence_ref` | Delete; resolve the winner against the sealed candidate prefix by reservation, ordinal, and transaction hash |
| `CompletedWalletNonce.completion_evidence_ref` | Delete; the retained `semantic_completion_key: EvmNonceCompletionKey` is the permanent row/operation identity |
| `CompletedRecoveryClosure.completion_evidence_ref` | Delete; the closure already retains the typed completion request/result lineage |
| `ReservedWalletNonce.observed_floor_ref` | Delete; compare the exact retained qualified floor and reservation state input directly |
| `CompletedWalletNonce.original_terminal_witnesses_ref` | Delete; validate the complete retained typed witnesses directly |

Delete `crates/storages/evm-postgres/src/support.rs::evidence_reference`, its schema seed and hash/
`ContentRef` imports, and all construction/revalidation calls in `authority.rs`. Add no evidence
table, evidence object, evidence column, or replacement opaque reference. Candidate reload must
require equality among the SQL primary key, request operation key, active-candidate operation key,
and the key derived from reservation plus ordinal. Reservation/completion reload validates the
retained typed closures and provider attestations directly.

These persisted JSON bytes change. Reset the current EVM wallet PostgreSQL baseline in commit 1:
edit `0001_wallet_authority.sql` in place, advance
`mfm.evm.wallet-authority-postgres.v1` to `mfm.evm.wallet-authority-postgres.v2`, regenerate every
wallet migration/catalog/constraint attestation, and reject old JSON fields through strict current
decoding. Advance the provider protocol from 3 to 4 in both the production provider and
`tests/wallet-authority-provider`; no v3 decoder or negotiation path remains. This work is complete
before commit 2, and commit 3 still leaves `crates/storages/evm-postgres/` byte-identical to commit
2.

Delete public production `HistoryObject::new(object_type, schema_id, json)` and generic
`HistoryObject::decode<T>()`. Keep arbitrary hostile-object creation only in explicit test support;
production uses `from_persisted`/`decode_persisted` with a concrete owner.

The current typed-object API is test-only scaffolding for a dependency-cycle reason:
`HistoryObjectPayload` is declared in `mfm-journal`, so lower-level spec/fact/value owners cannot
implement it without depending upward on journal. It currently has no production implementation.
Delete that trait everywhere. Introduce the one lower-layer `mfm_values::PersistedObjectPayload`
contract, owning object kind plus persisted shape/canonical bytes, and make journal's typed object
API consume it. Then delete `TestHistoryPayload`-only proof and every production fallback to the
arbitrary constructor. Add real implementations on every retained object owner; do not add a
journal forwarding trait or blanket “any Serialize” implementation.

Delete the same self-referential/cached-reference pattern from spec owners:

- `ClosedSumContract.closed_sum_contract_ref` and `ClosedSumContractPreimage`;
- `StructuredFactDescriptor.descriptor_ref` and `StructuredFactDescriptorPreimage`;
- `StructuredStateContract.state_contract_ref` and `StructuredStateContractPreimage`;
- `ExpansionPolicyContract.policy_ref` and `ExpansionPolicyContractPreimage`.

Those structs currently derive a schema over the full struct while hashing different preimage bytes
that omit the cached reference. Make each persisted payload self-reference-free. Pair it with its
exact typed `HistoryObject`/opaque checked object when callers need a reference, and obtain the
reference only from that object's `content_ref`. Update every program, certification, EVM,
portfolio, store, PostgreSQL, and test caller; add no replacement cached-ref field or forwarding
formula.

Delete `crates/kernel/facts/src/codec.rs` whole and remove its module declaration. Delete
`canonical_request_base64url`, `canonical_request_json`, the nested canonical document,
base64 encode/decode, repeated accessor reparsing, generic object/field parsers, and cached
canonical-byte fields inside fact query/subject/scalar/predicate types. Replace them with one direct
typed request/query representation; do not move the generic codec elsewhere.

From `replay/portable.rs`, delete:

- the current `PortableRunExportStream::schema_identity` descriptor whose generic
  `kind + CanonicalJsonTerminal payload` does not describe the closed record enum;
- `serde_json::Value` payloads, `PortableFrameKind`, erased `PortableFrame`, and
  `PortableBatchPayload`;
- all `serde_json::to_value`/`from_value` frame conversion and reparsing;
- the placeholder-first `PortableRunExport` construction and `placeholder_digest`;
- any remaining frame schema/identity, ordinal, predecessor/final/chain digest, repeated frame/byte
  count, seal-size convergence, or V2 branch/fixture;
- graph/frontier-only prior-run completeness as a substitute for executing the exact selector over
  supplied producer prefixes.

Replace the erased frame pair with one private closed `Batch | Seal` record enum under the one
complete-stream v3 identity. The descriptor names that exact enum and its framing bounds; the line
bound includes LF. Rename the retained semantic `closure_reference` to
`authorized_closure_digest` so it cannot be mistaken for stream byte identity. Replace arbitrary
string-payload boundary tests with typed record boundaries. Derive the closed persisted identity from
that concrete owner; do not hand-write another `schema_identity` descriptor in `portable.rs`.
`docs/design.md` still describes v2 ordinal/chain frames; delete that paragraph and write only the
implemented v3 contract.

### Test and documentation tombstones

Delete or replace—not rename—the following misleading evidence:

- `program_qualifier_is_not_a_public_trait.{rs,stderr}`;
- the current body and stderr expectation of `validated_append_cannot_be_counterfeited`;
- `incremental_reduction_matches_a_fresh_fold_at_every_prefix`, whose current body does not compare
  incremental and full state;
- manual-only attention tests that never construct `Manual`, `EntryClosable`, and Effect
  `Reassertable` inventory results;
- PostgreSQL tests coupled to removed fact summary columns, semantic configuration SQL, head-only
  cache validity, or pre-open split helpers;
- fixtures whose only purpose is a deleted generic factory, placeholder digest, inner fact codec,
  old configuration reference, or erased portable frame.

Do not recreate the deleted 5,409-line inline store test module. Replace it with smaller
boundary-focused targets arranged by qualifier, reducer/compiler equivalence, obligations,
mechanical backend, semantic open, and attention inventory.

Rewrite current documentation rather than recording old designs. In particular, delete the
477-line `docs/effect-entry-resolution.md` whole: it is an implementation-plan/rejected-alternatives
archive mislabeled as a current contract. Move only its short normative absorption and three-
resolution table into `docs/design.md`/`docs/run-execution.md`, remove links from the root README and
known gaps, then delete the file. Also delete the v2 portable paragraph from `docs/design.md`,
head-only cache claims, manual-only attention claims, observation-preflight descriptions, generic
hash claims, and PostgreSQL-semantic-validator claims. Historical explanations remain only in Git
history and this planning anchor.

Rewrite the root README architecture line, `crates/kernel/store/README.md`, and the module header in
`structured/mod.rs`; delete the misleading phrase `sole callback-free fold`. The current description
must say: owner qualification produces immutable typed history, callback-free `reduce_event`
interprets only that history, the compiler authors/compares records and projections, obligations
discharge external/current checks, semantic open audits projections, and backends apply sealed plans
mechanically. In `docs/design.md`, remove “complete records and append heads in physical chronology”
from the list of reducer products. Those are qualified-history/compiler concerns; the reducer owns
only compact semantic continuation. Preserve “callback-free” only as a property of `reduce_event`,
not as the name of the complete storage path.

### LOC and negative-delta acceptance

The reported repository-wide net reduction hid production growth by deleting old tests and the
recoverability plane. Compare like-for-like owner closures:

| Production closure | `d2d46752` | `a9ad9442f` | Wrong-direction delta | Final ceiling |
| --- | ---: | ---: | ---: | ---: |
| Semantic core | 5,341 | 6,110 | +769 | 5,341 |
| Structured store source, excluding dedicated tests/test support | 11,058 | 11,896 | +838 | 11,058 |
| Facts source, excluding `tests.rs` | 1,782 | 2,079 | +297 | 1,782 |
| PostgreSQL `src` plus destructive baseline | 6,668 | 6,747 | +79 | 6,668 |

The `d2d46752` semantic core is old `fold.rs + qualification.rs + canonical_append.rs`. The
`a9ad9442f` core is `qualification + reducer + compiler + coordinator + obligations + projection +
canonical_append`. The final core manifest must include every replacement semantic-owner helper,
including `validated_append`; adding a helper file, macro expansion, or facade cannot evade it.

Each final closure must meet its `d2d46752` ceiling and be net-negative relative to `a9ad9442f`.
Tests and documentation are counted separately so deleting evidence cannot subsidize production
growth. If a correctness requirement genuinely cannot fit a ceiling, stop for architect review with
a per-file responsibility/LOC explanation; the engineer may not self-waive or silently move code.

Record the exact file manifest, line count, and numstat for each closure. This shell establishes the
directory-wide closures; supplement `semantic_core_files` with every new helper that owns semantic
qualification/reduction/compilation/obligations/sealing:

```bash
set -euo pipefail
correction_baseline=a9ad9442fb0439b5334d3b107b9639d025ab1f58
# This is a two-tree comparison across the rewritten history, not an ancestry assertion.

semantic_core_files=(
  crates/kernel/store/src/structured/qualification.rs
  crates/kernel/store/src/structured/reducer.rs
  crates/kernel/store/src/structured/compiler.rs
  crates/kernel/store/src/structured/coordinator.rs
  crates/kernel/store/src/structured/obligations.rs
  crates/kernel/store/src/structured/validated_append.rs
)

# The diff manifest is the union of the correction baseline and final owner paths. In particular,
# it includes canonical_append.rs even though that file must not exist in the final tree. Add every
# final semantic-owner helper to both manifests above/below; omission is an acceptance failure.
semantic_diff_files=(
  crates/kernel/store/src/structured/qualification.rs
  crates/kernel/store/src/structured/reducer.rs
  crates/kernel/store/src/structured/compiler.rs
  crates/kernel/store/src/structured/coordinator.rs
  crates/kernel/store/src/structured/obligations.rs
  crates/kernel/store/src/structured/projection.rs
  crates/kernel/store/src/structured/canonical_append.rs
  crates/kernel/store/src/structured/validated_append.rs
)

audited_d2d_semantic_files=(
  crates/kernel/store/src/structured/fold.rs
  crates/kernel/store/src/structured/qualification.rs
  crates/kernel/store/src/structured/canonical_append.rs
)
audited_correction_semantic_files=(
  crates/kernel/store/src/structured/qualification.rs
  crates/kernel/store/src/structured/reducer.rs
  crates/kernel/store/src/structured/compiler.rs
  crates/kernel/store/src/structured/coordinator.rs
  crates/kernel/store/src/structured/obligations.rs
  crates/kernel/store/src/structured/projection.rs
  crates/kernel/store/src/structured/canonical_append.rs
)

mapfile -t structured_store_files < <(
  rg --files crates/kernel/store/src/structured --glob '*.rs' |
    rg -v '/test_support[.]rs$' |
    sort
)
mapfile -t fact_source_files < <(
  rg --files crates/kernel/facts/src --glob '*.rs' |
    rg -v '/tests[.]rs$' |
    sort
)
mapfile -t postgres_source_files < <(
  {
    rg --files crates/storages/postgres/src --glob '*.rs'
    printf '%s\n' crates/storages/postgres/migrations/0001_store.sql
  } | sort
)

# A final-file-only pathspec hides deleted production files from numstat. Build correction diff
# manifests from the union of the correction baseline and final tree instead.
mapfile -t structured_store_diff_files < <(
  {
    git ls-tree -r --name-only "$correction_baseline" -- crates/kernel/store/src/structured
    printf '%s\n' "${structured_store_files[@]}"
  } |
    rg '[.]rs$' |
    rg -v '/test_support[.]rs$' |
    sort -u
)
mapfile -t fact_source_diff_files < <(
  {
    git ls-tree -r --name-only "$correction_baseline" -- crates/kernel/facts/src
    printf '%s\n' "${fact_source_files[@]}"
  } |
    rg '[.]rs$' |
    rg -v '/tests[.]rs$' |
    sort -u
)
mapfile -t postgres_source_diff_files < <(
  {
    git ls-tree -r --name-only "$correction_baseline" -- \
      crates/storages/postgres/src \
      crates/storages/postgres/migrations/0001_store.sql
    printf '%s\n' "${postgres_source_files[@]}"
  } | sort -u
)

count_lines() {
  local total=0 path lines
  for path in "$@"; do
    test -f "$path" || continue
    lines=$(wc -l < "$path")
    total=$((total + lines))
  done
  printf '%s\n' "$total"
}

count_ref_lines() {
  local ref=$1 total=0 path lines
  shift
  for path in "$@"; do
    lines=$(git show "${ref}:${path}" | wc -l)
    total=$((total + lines))
  done
  printf '%s\n' "$total"
}

for path in "${semantic_core_files[@]}"; do
  test -f "$path"
done

test "$(count_ref_lines d2d46752 "${audited_d2d_semantic_files[@]}")" -eq 5341
test "$(count_ref_lines "$correction_baseline" "${audited_correction_semantic_files[@]}")" -eq 6110

semantic_lines=$(count_lines "${semantic_core_files[@]}")
store_lines=$(count_lines "${structured_store_files[@]}")
fact_lines=$(count_lines "${fact_source_files[@]}")
postgres_lines=$(count_lines "${postgres_source_files[@]}")

test "$semantic_lines" -le 5341
test "$store_lines" -le 11058
test "$fact_lines" -le 1782
test "$postgres_lines" -le 6668

require_net_negative() {
  local label=$1
  shift
  git diff --numstat "$correction_baseline"..HEAD -- "$@" |
    awk -v label="$label" '
      $1 ~ /^[0-9]+$/ && $2 ~ /^[0-9]+$/ { added += $1; deleted += $2 }
      END {
        printf "%s additions=%d deletions=%d\n", label, added, deleted
        if (deleted <= added) exit 1
      }
    '
}

require_net_negative semantic "${semantic_diff_files[@]}"
require_net_negative store "${structured_store_diff_files[@]}"
require_net_negative facts "${fact_source_diff_files[@]}"
require_net_negative postgres "${postgres_source_diff_files[@]}"
```

Measure persisted-identity deletion at commit 1, before the Runtime/store and documentation commits
can subsidize it. The audited production roots grew by 3,116 additions and only 480 deletions from
`d2d46752` to `a9ad9442f`. The raw baseline contains at most 269 inline-test lines in these files, so
the gate conservatively deducts all 269 from deletion credit whether or not they were touched. This
makes the required 1,100 credited production deletions executable without counting dedicated or
inline tests:

```bash
set -euo pipefail
identity_baseline=a9ad9442fb0439b5334d3b107b9639d025ab1f58
identity_commit=${IDENTITY_COMMIT:?set the exact commit-1 SHA}

test "$(git show -s --format=%s "$identity_commit")" = \
  'finish rust-owned persisted identity ownership'
# These are intentionally two tree endpoints on rewritten histories; a9ad9442f must not be an
# ancestor of the final branch.

identity_roots=(
  crates/kernel/canonical/src
  crates/kernel/program-derive/src
  crates/kernel/values/src
  crates/kernel/facts/src
  crates/kernel/journal/src
  crates/kernel/spec/src
  crates/kernel/certify/src
)
mapfile -t identity_diff_files < <(
  {
    git ls-tree -r --name-only "$identity_baseline" -- "${identity_roots[@]}"
    git ls-tree -r --name-only "$identity_commit" -- "${identity_roots[@]}"
  } |
    awk '
      /[.]rs$/ &&
      $0 !~ /\/tests\// &&
      $0 !~ /\/tests[.]rs$/ &&
      $0 !~ /_tests[.]rs$/ &&
      $0 !~ /\/test_support[.]rs$/
    ' |
    sort -u
)
test "${#identity_diff_files[@]}" -gt 0

git diff --find-renames=1% --numstat "$identity_baseline..$identity_commit" -- \
  "${identity_diff_files[@]}" |
  awk '
    $1 == "-" || $2 == "-" {
      print "binary path in persisted-identity Rust manifest" > "/dev/stderr"
      failed = 1
      next
    }
    $1 ~ /^[0-9]+$/ && $2 ~ /^[0-9]+$/ { added += $1; deleted += $2 }
    END {
      credited_deleted = deleted - 269
      printf "persisted identity production: additions=%d raw_deletions=%d credited_deletions=%d\n", \
        added, deleted, credited_deleted
      if (failed || credited_deleted < 1100 || credited_deleted <= added) exit 1
    }
  '
```

Portable, configuration, app, EVM, PostgreSQL, tests, fixtures, docs, manifests, and generated
artifacts receive zero credit in this identity LOC gate. Their cutovers and deletions remain
mandatory under their owner-specific gates.

This numeric gate is necessary but insufficient. Acceptance also requires every tombstoned symbol,
field, branch, SQL object, fixture, and false documentation claim above to be absent. A shorter
renamed monolith still fails; a correct result that keeps the obsolete implementation alongside the
new one also fails.

## One target design

There are exactly three state-bearing layers:

| Layer | Representation | Authority |
| --- | --- | --- |
| Immutable history | Canonical append-only batches, objects, and configuration revisions | Sole durable semantic authority |
| Verified semantic view | `VerifiedStructuredRun { history: Arc<QualifiedHistory>, reduced: ReducedRunState }` | Ephemeral, exact-prefix interpretation reproducible from history |
| Current projection | Run/configuration heads, tenant-fact routes/heads, and current Effect-attention membership | Mutable, disposable, compiler-produced index that is always compared back |

Qualification, reduction, obligation discharge, compilation, and persistence are transformations
between those layers, not additional authorities:

```text
raw immutable bytes
  -> owner qualification
  -> qualified immutable history
  -> callback-free reduce_event
  -> compiler comparison + closed obligation discharge
  -> finalized successor + sealed validated append
  -> mechanical atomic backend application
  -> route
  -> snapshot load + qualify + reduce + complete projection comparison
```

One owner must exist for each decision:

| Responsibility | Sole owner | Forbidden duplication |
| --- | --- | --- |
| Canonical syntax and hostile container bounds | `mfm-canonical` | Post-parse substitutes for hostile allocation bounds |
| Persisted field/tag/shape identity | The concrete Rust value owner | Name-selected schema factories or caller-selected badges |
| Program trust and immutable-history qualification | Certification registry plus store qualifier | Store-local verifier traits or fold-time schema rediscovery |
| Workflow legality and continuation | Pure `reduce_event` | Candidate preparation, replay, backend, or SQL reducers |
| Retained/current physical and prior-run checks | Closed obligation coordinator | Verifier callbacks inside reduction or partial discharge |
| Records, artifacts, assignment, assertion comparison, projection plans | One transition compiler | Reducer authoring or backend interpretation |
| Affine fact-scan authority | Post-commit coordinator, only after `NewlyCommitted` | Compiler/backend/precommit permit minting |
| Immutable persistence and current CAS | Backend | Matches on Runtime/journal semantic enums |
| Discovery answer | Qualified/reduced history | Index row, SQL predicate, or cached head alone |

No compatibility decoder, alias, dual path, backfill, later migration, receipt, or fallback is part
of the solution. Git history is the archive.

## Blocking defects and required corrections

### 1. The current reducer is still the old mixed fold

At the audited baseline:

- `crates/kernel/store/src/structured/reducer.rs` imports `CommittedBatch`, `AssignedRecord`,
  `RunRecord`, `HistoryObject`, `serde_json`, and physical verifier types;
- `FoldMachine` retains the certified program, raw objects, assigned records, batches,
  authorizations, observations, transitions, and journal heads;
- `reduce_event` is a method on `FoldMachine` and receives `ObligationDischargeScope` plus
  `&dyn PhysicalObligationChecker`;
- reducer code constructs retained framework objects and canonical JSON;
- `VerifiedStructuredRun` still contains `VerifiedFoldState { FoldMachine, DerivedProgram }`;
- full replay still enters through `fold_recorded_history`.

Replace this with private, disjoint products:

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
    // Semantic continuation only: cursor, bindings, leaves, semantic/physical
    // heads, facts, terminal outcome, attention, and reducer-owned indexes.
}

struct VerifiedStructuredRun {
    history: Arc<QualifiedHistory>,
    reduced: ReducedRunState,
}
```

Define `VerifiedStructuredRun` once in `structured/mod.rs`, where the public store API composes the
two layers. Its fields remain private. It has one private consuming constructor,
`VerifiedStructuredRun::from_finalized(history, finalized)`, callable only from `coordinator.rs` and
`semantic_open.rs`. Qualification constructs only `QualifiedHistory`; reduction constructs only
`ReducedRunState`; live commit and retained semantic open may pair them only from
`FinalizedReduction`. Neither qualification nor reducer owns or constructs the combined verified
product, and no test-support constructor exists.

`QualifiedHistory` owns immutable evidence needed by audit/export/purpose readers. It may contain a
structurally valid but semantically illegal sequence. `ReducedRunState` contains no raw bytes,
qualified batch, assigned record, `HistoryObject`, object store, serializer, backend, verifier,
capability, or authoring state.

The only transition rule is synchronous and capability-free:

```rust
fn reduce_event(
    context: &QualifiedRunContext,
    previous: &ReducedRunState,
    event: &QualifiedEvent,
) -> Result<PendingSemanticStep>;
```

It covers admission and every later event. Delete separate admission/authorization/observation/
transition semantic preparation. Frontier and Effect attention are reducer products; broad
`_ => BlockedIntegrity` fallbacks for supposedly impossible execution-kind/leaf combinations are
invalid history, not synthetic state.

Admission starts from one reducer-owned `ReducedRunState::empty(context)` seed. It is not a
persisted genesis snapshot, candidate, callback, or second admission reducer. Delete the current
`ReducerState` name and representation rather than retaining two compact-state types.

### 2. The typestate currently proves nothing

At the audited baseline, `PendingSemanticStep` and `ComparedReduction` each wrap only the same
`DerivedProgram`. `compare_recorded` compares assertions borrowed from a batch with that same batch,
and `discharge` ignores its scope and always succeeds.

Implement a consuming boundary with genuinely different private states:

```text
PendingSemanticStep
  -> compiler compares preview/recorded plans and retained assertions
  -> ComparedReduction
  -> obligation coordinator consumes and discharges the complete set
  -> FinalizedReduction
```

Requirements:

- `PendingSemanticStep` is non-`Clone`, coordinate-free, unsealable, exposes no successor, and owns
  `UnboundSuccessor`, typed record/artifact intents, `TenantFactRequirement`, semantic facts, and one
  closed obligation set.
- Intent preview may only borrow it to author candidate bytes.
- `ComparedReduction` can exist only after the compiler independently derives assignments,
  compares intent and recorded unbound plans, and matches every retained assertion and object.
- Discharge consumes the whole `ComparedReduction` exactly once. Retained checks always run first;
  a new candidate then runs current checks. There is no `CurrentOnly` or partial result.
- `FinalizedReduction` is the only type exposing the bound `ReducedRunState` and the only input from
  which `VerifiedStructuredRun` or `ValidatedRunAppend` can be constructed.
- No `finish(DerivedProgram)` or crate-internal constructor may bypass this chain.

The compiler owns serialization, deterministic records/artifacts, record assignment, object suffix,
assertion comparison, current projection, `TenantFactProjectionPlan`, and data-only
`FactScanPermitSpec`. It does not mint the live permit.

Retain `PhysicalObligationChecker` and its seal as the external capability owned by the obligation
coordinator. What must disappear is any checker callback inside the reducer, any partial/current-
only discharge mode, and any way to invoke the checker below the closed obligation boundary.

### 3. Candidate authoring does not use the same rule as replay

`QualifiedEvent::Intent` at the audited baseline is dead code carrying an already assigned
`CommittedBatch`. Transition preparation independently walks and mutates the program state; other
candidate paths manually construct records and reduce only the recorded form.

Every event family must use this sequence:

```text
stable normalized Runtime intent
  -> lookup (run_id, append_request_id)
       present: qualify/replay retained attempt, compare normalized intent,
                reload actual current state, return ExistingSame or conflict
       absent: continue
  -> reduce QualifiedEvent::Intent from exact predecessor
  -> resolve required tenant frontier
  -> compiler preview authors deterministic assignments/bytes
  -> owner-qualify the exact candidate bytes
  -> reduce QualifiedEvent::Recorded from the same predecessor
  -> compare independent preview/recorded plans and all assertions/objects
  -> discharge RetainedAndCurrent obligations
  -> seal ValidatedRunAppend
  -> backend reclassifies the stable key under locks and applies mechanically
```

The present lookup occurs before reading a current run/tenant frontier or authoring bytes. An absent
lookup grants no authority; the append transaction must classify the same key again under lock.

### 4. Stable retry resolution is too late

The current `resolve_append(run_id, append_request_id, candidate_digest)` requires a candidate that
has already been authored. Transition and authorization paths read the current tenant fact frontier
before candidate construction. A lost acknowledgement followed by frontier `N -> N + 1` can
therefore re-author different bytes before finding the original append.

Add a mechanical backend read:

```text
lookup_append_attempt(run_id, append_request_id)
```

It returns the exact retained append/prefix needed for store-owned qualification and normalized
intent comparison. It performs no semantic interpretation and no mutation.

For `ExistingSame`, qualify and return the actual current snapshot. Never release or cache the
candidate-built historical successor. `ExistingSame`, stale, conflict, acknowledgement ambiguity,
and replay release no fact permit. Only `NewlyCommitted` combines a `FactScanPermitSpec` with the
backend source/verifiers to mint the affine `FnOnce` capability.

### 5. The validated append is not an internal proof boundary

`crates/kernel/store/src/structured/canonical_append.rs` remains, and its crate-private constructor
accepts an ordinary `CommittedBatch` plus a freely constructible projection mutation.

Delete that file and replace it with `validated_append.rs`. The target ingress is conceptually:

```rust
struct ValidatedRunAppend {
    committed: CommittedBatch,
    run_projection: RunProjectionPlan,
    tenant_fact_plan: TenantFactProjectionPlan,
}

struct RunProjectionPlan {
    expected: Option<RunCurrentProjection>,
    successor: RunCurrentProjection,
}

enum TenantFactProjectionPlan {
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

Construction consumes `FinalizedReduction` and binds:

- batch run identity, exact predecessor and successor head;
- admitted tenant;
- reducer-produced Effect attention membership;
- exact expected current projection;
- assigned fact publication reference/order or barrier frontier;
- exact object closure.

`RunProjectionPlan` is a sealed compiler product, not a rename of the current freely constructible
`RunProjectionMutation`. Delete the old type, constructors, conversions, and all callers before
adding this replacement; its expected and successor values must be derived from the independently
compared reduction.

`ValidatedConfigurationAppend` similarly carries the exact expected/successor heads and is produced
only after the shared configuration verifier. Compile-fail tests must prove downstream code cannot
construct either command or extract the batch while silently dropping a required plan.

### 6. Backends still act as semantic authorities

PostgreSQL currently matches `TenantFactCoordinate`, inspects
`RunRecord::StateTransitionCommitted`, checks facts, constructs `PendingTenantFactPublication`, and
advances lifetime summaries. Memory independently repeats fact/configuration rules. PostgreSQL
configuration DML invokes `verify_configuration_history` and schema SQL uses JSON/window semantics
as another reducer.

Backends may only:

- strictly decode physical rows into checked IDs and bounded canonical envelopes;
- perform stable append lookup;
- acquire canonical run-then-tenant locks;
- compare exact append identities, predecessors, current projections, and supplied plans;
- insert immutable batches, objects, configuration revisions, and fact routes;
- CAS minimal run/configuration/fact heads;
- scan current attention routes;
- enumerate authoritative/projection key unions for qualification.

Backends must not match `RunRecord`, `StateLeaf`, `StructuredFrontier`, `AccessKind`,
`ObservationOutcome`, `TenantFactCoordinate`, entry budgets, settlement, or fact contents to choose
behavior.

For tenant facts:

- `None` takes no tenant lock and mutates nothing;
- `Barrier` locks and requires the exact frontier but mutates nothing;
- zero is absence of a `tenant_fact_heads` row;
- first `Publish` requires absent/zero, inserts order 1, and creates the positive head;
- later `Publish` requires exact `N`, inserts `N + 1`, and CASes the head;
- exact old retries validate their immutable route but never reapply it.

Delete `publication_count`, `minimum_order`, `maximum_order`, persistent zero heads,
`PendingTenantFactPublication`, `TenantFactRouteSummary`, lifetime aggregate probes, backend record
interpretation, and SQL configuration-chain reduction.

### 7. Complete projection compare-back is absent

`StructuredRunSnapshot.current_projection` is loaded but both store load paths compare only
`snapshot.head`. The Runtime cache is also validated by head alone. This permits a same-head
attention false negative or wrong tenant to hide a run indefinitely.

Use one snapshot value:

```rust
struct StructuredRunSnapshot {
    history: RawRunHistory,
    current_projection: RunCurrentProjection,
}
```

Delete the separate `StructuredRunSnapshot.head`, the backend/writer `current_head` method, and all
head-only adapter probes. The current projection is the single physical compare value; accepting a
second head field would preserve the memory/PostgreSQL parity split.

Every ordinary load and cache reuse must derive and compare the complete projection:

- run ID;
- tenant scope;
- journal sequence and digest;
- `has_effect_entry_attention`.

A same-head mismatch is corruption. A route whose head changed between scan and load is a live-sweep
race and may be skipped. A false positive route is never evidence; a false negative must be found by
semantic open rather than waiting for a scan that cannot select it.

### 8. Semantic open `S0` does not exist

Current assembly synchronously splits the store and returns Runtime/readers. PostgreSQL openers
construct backends and call it; configuration maintenance returns a writer without semantic
qualification.

Create `structured/semantic_open.rs` and make
`qualify_and_open_structured_store(...) -> Result<OpenedStructuredStore>` the sole store-layer async
one-shot transition. It consumes an unsplit structurally qualified session/backend bundle, the
complete `QualifiedProgramRegistry`, and the target/release fence.
`AdmissionVerificationRegistry`, `CertifiedProcessRegistry`, and `RuntimeAssemblyToken` remain
inseparably coupled until all semantic qualification succeeds.

Phase 1 establishes only physical usability:

- migration ledger and schema version;
- relation/constraint/index/executable/ACL manifests;
- exact database/schema/store identity, store scope, epoch, fence generation, release epoch;
- role shape and bounded row decoders.

No usable handle escapes Phase 1.

Delete the public synchronous `assemble_with_backend`/`assemble_structured_runtime` bypasses,
direct pre-audit `StructuredRunStore::new(...).split()` assembly, and maintenance/open paths that
return configuration or Runtime capabilities before this transition. Backend-specific application
constructors may acquire physical sessions and then call this entrypoint, but may not implement or
name a second semantic-open operation. Test support must use the same qualified opener rather than
a privileged shortcut.

Phase 2 uses one pinned PostgreSQL `REPEATABLE READ`, read-only snapshot `S0` for all run-history
decisions:

1. Enumerate the union of authoritative run IDs and projection run IDs in bounded pages.
2. Load each raw history, objects, current run projection, fact routes/heads, and recursive source
   prefix from that same snapshot.
3. Qualify each immutable prefix through the concrete certification registry.
4. Replay every recorded event through `reduce_event`, compiler comparison, and `RetainedOnly`
   discharge.
   Prior-run fact completeness must recompute the retained selection from the exact supplied source
   prefixes using the same selector as live execution; graph/frontier coverage alone is insufficient.
5. Compare every current run projection in both directions.
6. Recompute every fact plan and compare route/head state globally in both directions.
7. Reject missing, invented, lagging, leading, rewound, duplicate, non-dense, wrong-coordinate, or
   wrong-tenant projections.
8. Complete the snapshot without exposing a capability.

Use one exact-prefix memo:

```text
programs: (operation_id, certified_root_ref) -> Arc<CertifiedProgram>
prefixes: (run_id, through_journal_head) -> Visiting | Verified(Arc<VerifiedStructuredRun>)
```

Recursive `Visiting` re-entry is an invalid source cycle. A verified exact/earlier prefix may be
reused, but every consumer request still charges its logical work bounds.

Configuration uses its own pinned snapshot and the one shared Rust verifier. An opener returning
both capabilities exposes neither until both qualifications succeed.

After semantic snapshots complete, re-read the target authority in a fresh transaction and require
the original database identity, schema/store target, fence generation, release epoch, and admitted
release. A mismatch discards every qualified product. Same-release appends between `S0` and this
fresh check are allowed only through sealed validated commands and are checked on first use.

Normal open never repairs corruption. A future rebuild, if ever added, is a separate exclusive
deployment operation and not part of ordinary application authority.

### 9. Effect attention is incorrectly derived from the frontier

At the audited baseline, `EntryClosable` and `Reassertable` correctly have continuation frontier
`Actions`, while `has_effect_entry_attention` is set only for `PossibleEntry`. Consequently the
inventory omits the exact absorbing states that need timely recovery.

Attention is an orthogonal reducer product:

```rust
struct EffectEntryAttention {
    subject: EffectEntrySubject,
    resolution: EffectEntryAttentionResolution,
}

enum EffectEntryAttentionResolution {
    Manual,
    CloseThenReassert,
    Reassert,
}
```

Required table:

| Current Effect leaf | Continuation | Attention |
| --- | --- | --- |
| `Authorized` without remaining absorption | `PossibleEntry` | `Manual` |
| `EntryUnknown` without remaining absorption | `PossibleEntry` | `Manual` |
| `EntryClosable` | `Actions` | `CloseThenReassert` |
| Effect `Reassertable` | `Actions` | `Reassert` |
| Every other valid leaf | Existing continuation | none |

`StateLeaf::Reassertable` must carry the immediately preceding `access_attempt_id` as well as the
next ordinal so the attention subject is exact. Read may use the leaf for deterministic recovery but
never produces Effect attention.

Derive `has_effect_entry_attention` from `reduced.effect_entry_attention.is_some()`, never from
`frontier == PossibleEntry`. The reader re-derives and returns both subject and resolution. App,
CLI, and REST serialize exactly `manual`, `close_then_reassert`, or `reassert`.

Keep the current one-row-per-run boolean route and partial index. Do not persist the subject or
resolution, add an attempt index, or materialize a scheduler queue.

### 10. Observation qualification remains a duplicate semantic path

Delete the complete preflight family without aliases:

- `ObservationEventQualification` and any renamed `ObservationQualification`;
- `qualify_observation_event` and `qualify_invoked_observation_event`;
- `PendingObservation`;
- Runtime history-port and store-adapter methods for the preflight;
- separate supersession preflight and its Ready/ExistingSame/fault branches;
- `InvalidSupersessionEvidence` and every preflight-only cached-successor outcome;
- duplicate physical checks performed before the one recorded candidate pipeline.

Observation, closure, reassertion, and settlement all use the same `commit_event` coordinator and
the same intent/recorded reducer/compiler/obligation path. Existing-same classification happens by
stable append lookup, not a separate logical observation oracle.

### 11. Persisted-schema and identity ownership is still incomplete

This work is prerequisite to the Runtime cutover because qualified history must consume honest
owner contracts.

#### Direct fact request

`FactSelectionRequest` still persists `canonical_request_base64url` containing another canonical
document, and a string-selected open schema does not bind its real fields or bounds.

Persist one direct typed request/query representation. Its concrete fields, tags, literals, query
bounds, and selector contract participate in its owner-derived `SchemaId`. Delete the inner wrapper,
base64 parser/encoder, `selection_schema_id(name)`, old wire fixtures, and
`crates/kernel/facts/src/codec.rs`. Do not relocate its parser, base64 wrapper, nested-document
codec, registry, or field-extraction helpers under another module. The concrete owner uses ordinary
typed canonical serialization directly. Reject old wrapped JSON as unknown/invalid; add no decoder.

`FactSelectionQuery`, `FactScalar`, `FactSubject`, and `CanonicalFactPredicate` must retain direct
typed fields rather than cached canonical-byte fields. Accessors operate on those fields without
reparsing an inner document; canonical encoding happens once at the owner boundary.

#### Typed `HistoryObject`

Production still uses public arbitrary `HistoryObject::new(object_type, schema_id, json)` and
generic `decode<T>()`. Existing typed APIs are scaffolding used only by tests.

Move every production object to an owner implementing a typed persisted-payload contract that owns
both object type and schema. Production construction/decoding must be equivalent to:

```text
HistoryObject::from_persisted(&typed_owner_value)
HistoryObject::decode_persisted::<TypedOwner>()
```

Delete public arbitrary production constructors/decoders and the generic factories
`structured_content_ref`, `typed_content_ref`, `component_kind_schema_id`,
`canonical_document_schema_id`, `certification_document_schema_id`, `wallet_descriptor_ref`, and
`framework_object`. Replace the bespoke `CertifiedProgramRoot::content_ref` and forwarding
`CertifiedProgramDocument::content_ref` with reference acquisition from the concrete retained
typed root object. Keep hostile arbitrary-object construction only behind explicit test support.

Give every known certification, component-contract, framework outcome/claim, fact, and wallet
document its concrete persisted owner shape. An open terminal schema is allowed only for genuinely
external opaque bytes. An empty marker type whose schema is applied to an unrelated serialized
value is still caller-selected identity and must be deleted.

#### Semantic hash owners

`mfm_journal::structured::domain_content_digest(domain, value)` remains public and production EVM
callers choose domains dynamically. Replace every call with a named typed owner operation fixing the
domain and exact preimage type, add owner-local goldens and one-field hostile mutations, then delete
the generic helper. Delete every shape-erased generic candidate/attempt/record/commit helper: each
surviving journal identity has one named journal-owned preimage and one fixed owner operation. There
is no “substitution is impossible” generic escape hatch.

The final `docs/persisted-public-surfaces.md` must be current-owner-only and make its existing claim
that no caller-selected domain helper exists true. Historical deleted-name disposition remains in
Git history and this handoff only; do not preserve it in current documentation.

#### One configuration identity

The current `revision_ref` hashes a `RevisionPreimage`, while app admission creates a second
`HistoryObject` from the full self-referential `ConfigurationRevision` bytes.

Replace this with one persisted `ConfigurationRevision` payload containing the exact routing key,
sequence, predecessor, append request, value contract/reference, and canonical configured value.
The payload does not serialize `revision_ref`. It implements the typed persisted-schema and
lower-level persisted-object payload contracts and fixes the configuration admission object type.

Use one opaque checked product:

```text
ConfigurationRevisionObject {
  object: HistoryObject,
  decoded: ConfigurationRevision,
}

content_ref() = object.content_ref
```

It contains no cached or independently supplied identity. `ValidatedConfigurationAppend` owns this
exact object, derives its expected head from the payload's `predecessor_ref`, and derives its
successor head from `object.content_ref`. Configuration heads and successor predecessor references
use that same content reference. `VerifiedConfiguredValue` retains the exact qualified object, and
app admission clones that object verbatim instead of reserializing the revision.

Delete `RevisionPreimage`, the serialized field/descriptor/accessor, the bespoke `revision_ref(...)`
formula, caller-supplied validated-append heads, and the second app-side construction. PostgreSQL
reconstructs the fixed-kind raw object and lets the shared store qualifier decode the payload; row
route/sequence/predecessor/append coordinates are physical comparison data, not another identity.
Delete relational value-contract/value mirrors and semantic SQL checks that have no physical
lookup/CAS purpose. Mutating any payload field must change the one object reference.

#### Portable v3 closed stream

Portable v3 currently uses an open `kind + serde_json::Value` record schema and has a one-byte
framed-record bound disagreement between descriptor and codec.

Give the complete NDJSON artifact one `CanonicalJsonLines` persisted identity whose descriptor
contains a closed private `Batch | Seal` record union and whose record-byte bound includes the LF
exactly as the codec enforces it. No frame receives a `SchemaId` or `ContentRef`. Encoding returns
one product containing exact bytes plus their stream-level `ContentRef`; offline verification
requires the expected complete-stream reference and checks it before decoding.

Delete erased `serde_json::Value` payloads, frame identity/chain/count fields, derivative digest
placeholders, and any v2/current ambiguity. Preserve the semantic closure digest as a separate
authorization identity, not a byte identity.

#### Canonical hostile-input bounds

`mfm-canonical` declares string, array, map, and key bounds but currently enforces only total bytes
and depth before allocating nested containers. Enforce every declared bound inside the canonical
parser/visitor at hostile ingress. Exact-limit cases pass; limit-plus-one fails before unbounded
allocation. Downstream value validation remains semantic defense, not a substitute.

#### Certified admission-policy ownership

Retain `CertifiedProgramComponents::qualified_entry_point_admission_policy_ref` only in the spec/
certification owner and its certification tests. It closes over the exact policy component and
outbound dependencies.

`RunAdmitted` must contain only `certified_program_ref`; injected legacy policy/audit fields are
unknown. Delete/keep globally forbidden `CertifiedProgramAuditRefs`, `audit_refs`,
`validate_admission_audit_refs`, and `certified_program_root_ref`. Tests must reject missing,
substituted, changed, or internally consistent alternate policy objects during fresh registry
verification.

Narrow the ownership census to exactly:

- `crates/kernel/spec/src/structured.rs`;
- `crates/kernel/certify/src/structured.rs`;
- `crates/kernel/certify/tests/structured_certification.rs`.

Any other production or persisted surface carrying
`qualified_entry_point_admission_policy_ref` fails acceptance.

### 12. Tests currently do not prove the claimed equivalence

The existing `incremental_reduction_matches_a_fresh_fold_at_every_prefix` only performs fresh folds
and discards the results. The attention suite contains only negative Pure/zero-limit/foreign-tenant
cases. The compile-fail append test proves external visibility, not the internal proof chain.

Replace names-only evidence with the regression matrix below.

## Required regression matrix

### Qualification

- Canonical syntax, size/depth/container bounds, run identity, store scope/epoch, sequence,
  predecessor, append request, candidate/commit/record hashes, object identity/type/closure, direct
  references, five-family decoding, and certified-root correspondence are each hostile-mutated.
- A structurally valid but semantically illegal history can become `QualifiedHistory` and is then
  rejected by reduction, proving the boundary is honest.
- Program memo keys include `(operation_id, certified_root_ref)` and still compare exact root and
  authored bytes on hits.
- No public/test-support constructor can counterfeit qualified history or mix a qualified context
  with another program.

### Pure reducer and compiler

- Full recorded reduction and incremental reduction produce exactly equal compact state at every
  prefix; compare the values, do not merely execute both.
- Cover admission/zero-state closure, Pure success/failure, lexical binding, match, fragment,
  fan-out/join, Read lifecycle, Effect lifecycle, fact publication/barrier, and terminal closure.
- Cover illegal duplicate outstanding access, wrong ordinal/subject/outcome, illegal closure
  placement, and every impossible execution-kind/leaf combination.
- For every event family, intent preview and requalified recorded reduction produce equal unbound
  plans and successor semantics.
- Full/incremental equality includes cursor, frontier, heads, bindings, terminal outcome, attention,
  obligations, expected records/artifacts, fact plan, and projection.
- One-field mutations of record ref, semantic head, object bytes/ref/closure, fact identity,
  outcome, assignment, or cross-run coordinate fail exact comparison.
- Compile-fail tests prove preview/unbound/compared values cannot construct verified state, cache
  values, append commands, or permits and cannot bypass whole-set discharge.

### Obligations

- Retained verification always runs before current verification.
- Recorded replay uses retained-only checks and cannot invoke current checks.
- A new candidate cannot seal if any retained or current physical binding, supersession, or prior-
  run completeness obligation fails.
- Portable and live prior-run selection use the same selector and exact source prefixes.
- No partial discharge result or current-only scope exists.

### Mechanical backend and idempotency

Run the same conformance suite against memory and PostgreSQL:

- a new append atomically installs immutable batch/objects plus supplied run and fact projections;
- stale expected run or tenant state changes nothing;
- injected failure at batch, object, run head, fact route, or fact head rolls back every surface;
- exact current and historical retries return `ExistingSame` without DML, projection rewind,
  historical cache installation, or permit release;
- acknowledgement loss at tenant frontier `N`, followed by publication `N + 1`, resolves the old
  stable request before recompilation;
- same request ID with a different normalized intent conflicts;
- absent prelookup raced by an identical append is reclassified under the append locks;
- acknowledgement ambiguity never reapplies a historical after-image;
- same journal head with wrong tenant or attention is corruption, not a cache hit;
- concurrent run successors serialize and concurrent tenant publications remain dense/gap-free;
- memory and PostgreSQL reject the same malformed supplied projection bindings.
- a type-level backend conformance test proves historical `load_prefix` accepts the exact requested
  `JournalHead` and returns that immutable prefix without consulting the current projection/head;
  a sequence-only request or head-only helper does not compile.

### Semantic open and projection corruption

Reject each independent mutation:

- missing, invented, lagging, leading, rewound, wrong-tenant, wrong-head, or wrong-attention run
  projection;
- canonical run without a head and head without a canonical run;
- missing, invented, duplicate, non-dense, wrong-coordinate/order/tenant fact route;
- missing, invented, rewound fact head, including a persistent zero row;
- configuration revision/head mismatch in either direction;
- corrupt retained history whose projection happens to look valid;
- missing attention membership for reducer-positive state;
- invented attention membership for reducer-negative state.

Also prove no handle escapes between physical and semantic open, prefix cycles fail, shared producer
prefixes are reduced once without bypassing bounds, configuration failure blocks combined openers,
final target/fence/release changes fail, same-release sealed appends across the final check remain
valid, and normal open performs no repair.

### Effect attention and pagination

- terminal Effect ambiguity returns `Manual`;
- `EntryClosable` returns `CloseThenReassert` while frontier remains `Actions`;
- Effect `Reassertable` returns `Reassert` naming the exact preceding attempt;
- Read `Reassertable` never enters attention;
- authorize -> crash -> close -> reassert twice -> budget exhaustion remains a named regression;
- every closing audit record uses the kernel closure-adapter discriminator;
- two closing racers produce one append and the loser reloads authoritative state without a
  component-attributed `CandidateRejected` fault;
- minimum lineage head and previous physical binding remain `None` for `Reassertable`;
- a forged positive route is never evidence;
- a recovered `EntryUnknown -> Returned -> Closed` run has no route;
- arbitrarily many historical attempts consume at most one current route;
- hundreds of recovered runs plus one attentive run visit one partial-index row and use
  `run_history_heads_effect_entry_attention_v1`;
- `limit = 1` over multiple attentive runs is ordered, duplicate-free, skip-free in a quiescent
  sweep, and has the correct sentinel continuation;
- a changed head between scan/load is skipped while same-head disagreement is corruption;
- behind-cursor arrivals wait for the next sweep, sparse/empty pages may continue, and restart
  rediscovers unresolved attention;
- cursor tenant, version, encoding, size, unknown fields, and obsolete shapes are hostile-tested.

### Public, replay, and EVM boundaries

- app/CLI/REST expose only redaction-safe subject plus
  `manual | close_then_reassert | reassert`;
- attention-list permission cannot read/audit/drive/export/replay and those grants cannot list;
- no record body, private implementation identity, certificate bytes, drive capability, or cross-
  tenant data leaks;
- online and offline replay agree on every prefix and use the same prior-run selector semantics;
- portable wrong schema/digest fails before semantic decode, and framing/limit/tag mutations fail;
- EVM retains unknown-on-absence, kernel closure discrimination, budget-bounded reassertion, and
  restart recovery;
- provider v4/wallet PostgreSQL v2 reject old JSON and direct mutation of retained
  request/state/key/result/proof fields;
- active candidates reject a foreign or wrong-ordinal operation key, replacement permits reject a
  foreign predecessor operation key, and terminal winner ordinal/hash substitution fails;
- reservation/completion reload rejects request, floor, state-input, lineage, outcome, witness, or
  provider-attestation substitution by comparing the complete typed closures, without any opaque
  evidence reference;
- the two-process in-flight absorption limitation and deployment sweep cadence remain explicit.

## Exact deletion and ownership scope

Delete without facade, alias, or renamed compatibility path:

### Runtime/store

- `fold_recorded_history`, `VerifiedFoldState`, and `FoldMachine`;
- reducer imports/fields for raw batches, assigned records, history objects, serialization,
  backends, verifiers, callbacks, or IO;
- the cosmetic existing `PendingSemanticStep`, `ComparedReduction`, and unconditional `discharge`;
- reducer-side component decoding and `validate_program_value_schemas` rediscovery;
- manual admission/transition preparation and duplicate record-family reducers;
- observation/supersession preflight family described above;
- `crates/kernel/store/src/structured/canonical_append.rs`;
- `crates/kernel/store/src/structured/mutation.rs`, `assembly.rs`, and `projection.rs` without
  replacement facades;
- the complete audited bodies of `reducer.rs`, `coordinator.rs`, and `obligations.rs` before their
  smaller single-owner replacements are written;
- `crates/kernel/store/tests/ui/fail/program_qualifier_is_not_a_public_trait.rs` and its `.stderr`;
  the fixture fails on an invented import and does not prove the concrete certification boundary;
- head-only cache probes, optional prepared successors, and cached candidate successor on
  `ExistingSame`;
- `StructuredRunSnapshot.head`, the backend/writer `current_head` operation, and public synchronous
  `assemble_with_backend`/`assemble_structured_runtime` paths;
- late-only `resolve_append`/`resolve_attempt`; use the one stable lookup operation both before
  authoring and when resolving acknowledgement ambiguity;
- precommit live `FactScanPermit` construction;
- every projection receipt/receipt digest/historical after-image concept;
- `EffectEntryAttentionEvidence`/`EffectEntryAttentionPageEvidence`, manual-only classification,
  the duplicate cursor body version, and derived/public-field attention serialization;
- `RunAccessGrant`/`RunAccessPolicy` and all aliases; only
  `ApplicationAccessGrant`/`ApplicationAccessPolicy` remain;
- stale `RegistryProgramVerifier` comments and obsolete UI fixture names.

### Memory/PostgreSQL

- all backend interpretation of Runtime/journal semantic enums;
- independent tenant-fact and configuration reducers;
- `publication_count`, `minimum_order`, `maximum_order`, their SQL constraints/parsers/queries, and
  persistent zero fact heads;
- `PendingTenantFactPublication`, `TenantFactRouteSummary`, and aggregate probes;
- SQL window/JSON configuration-chain semantics;
- any attempt-table code, page-local dedup, or historical attention derivation that reappears;
- synchronous/public assembly paths that can split capabilities before semantic open;
- temporary catalog-hash emitters once final hashes are pinned.

Keep/reinforce in the v7 baseline:

- one `run_history_heads` row per run;
- tenant scope and attention membership on that row;
- exactly one partial attention index;
- an exact `(run_id, head_sequence)` foreign key to the current immutable batch;
- immutable batch/object rows and minimal positive tenant-fact/configuration heads.

Rewrite `0001_store.sql` in place in commit 2; the mandatory column/constraint deletion and exact-head
foreign key guarantee a new destructive baseline. Add no `0002`. Regenerate relation, constraint,
index, executable/trigger, ACL, migration-ledger, and SQLx evidence from fresh schemas.

### Persisted identities/codecs

- public caller-selected `domain_content_digest`;
- journal `persisted_identity(name, shape)`, `DomainEnvelope<T>`, and
  `domain_envelope_digest(domain, value)`;
- EVM `wallet_policy_schemas` and all of its generic name/field-selected schema builders;
- `crates/kernel/facts/src/codec.rs` whole; do not relocate its generic parser/registry;
- production `structured_content_ref`, `typed_content_ref`, `wallet_descriptor_ref`,
  `framework_object`, string-selected schemas, and arbitrary object construction/decoding;
- `canonical_request_base64url` and the inner fact request codec;
- `component_kind_schema_id`, `canonical_document_schema_id`,
  `certification_document_schema_id`, and bespoke certified-root/document `content_ref` methods;
- duplicate configuration revision preimage/object identity;
- portable erased record payloads, false frame identities, derivative chain/count placeholders, and
  any v2 compatibility;
- obsolete EVM evidence field names from current documentation as well as code/fixtures;
- `docs/effect-entry-resolution.md` whole, its inbound links, and current docs that describe deleted
  mechanisms as history. Historical disposition belongs only to Git history and this file while it
  exists.

Do not delete or rename the certified-root-owned
`qualified_entry_point_admission_policy_ref`. Delete only copies outside its exact owner census.

## Logical commit sequence

The final local branch must be rewritten into ordered coherent commits. Do not append dozens of
small fixups to the already misleading completion history. Preserve the private backup ref until
the rewritten tree and final evidence are accepted. The planning anchor is the direct child of
`56c260ba492ef5de71e3ef82250295cd583a2868`; none of the superseded implementation commits is an
ancestor. Tag the fully verified working implementation temporarily, compare its tree object with
the rebuilt final `HEAD`, and delete that temporary tag/working branch only after equality. Keep only
the private pre-cutover backup ref.

Before the three implementation commits, commit this file alone as
`plan the 10c53f2a runtime-store correction`. That planning-anchor commit records the complete
problem and is intentionally removed by commit 3; it is not a compatibility or implementation
commit.

### Commit 1: `finish rust-owned persisted identity ownership`

One inseparable persisted-schema/hash cutover:

- enforce canonical hostile container bounds;
- delete the recoverability annex/generator/task plane and retain only owner-scoped limits;
- finish the owner-derived shape vocabulary;
- implement direct typed fact request/query bytes;
- migrate every production `HistoryObject` construction/decode to its concrete owner;
- replace caller-selected semantic hash calls, `DomainEnvelope`, and generic journal identity
  factories with fixed-domain typed owner functions;
- replace the EVM wallet policy schema/JSON builders with four concrete persisted document owners;
- cut configuration to one persisted payload/reference;
- make portable v3 a closed typed JSON-lines stream with one exact byte identity;
- preserve the certified admission-policy component only in its exact owner closure;
- update all program roots, nested references, batches, fixtures, public bytes, and current-only
  persisted-surface documentation;
- regenerate affected wallet/provider/PostgreSQL evidence if bytes change;
- delete every superseded factory, wrapper, field, helper, fixture, and document in the same commit.

No producer may emit an old identity while another consumer expects a new one. Add no legacy
decoder.

### Commit 2: `replace runtime storage with three qualified layers`

This is deliberately large and inseparable. In one coherent cutover:

- implement `QualifiedRunContext`, `QualifiedHistory`, `QualifiedBatch`, qualified intent/recorded
  events, compact `ReducedRunState`, and honest `VerifiedStructuredRun`;
- replace `FoldMachine` with the one pure `reduce_event`;
- implement real consuming compiler/comparison/obligation typestate;
- move all record/artifact/assignment/projection compilation out of the reducer;
- add stable pre-authoring `lookup_append_attempt`;
- seal validated run/configuration appends from `FinalizedReduction`;
- move permit minting to the exact `NewlyCommitted` post-commit branch;
- delete observation and other duplicate semantic paths;
- make memory and PostgreSQL mechanically apply supplied run/fact/configuration plans;
- add complete projection compare-back on loads/cache use;
- implement async unsplittable semantic open at `S0` and final fence/release validation;
- implement orthogonal three-resolution Effect attention across reducer, current projection,
  inventory, app, CLI, and REST;
- rewrite the PostgreSQL v7 baseline and regenerate all manifests/SQLx evidence unconditionally;
- update `docs/design.md`, `docs/architecture.md`, `docs/run-execution.md`, app/CLI/REST docs, and
  root links to the one implemented runtime/store/attention contract; move the small normative
  Effect table and delete `docs/effect-entry-resolution.md` in this commit;
- land the complete reducer/backend/open/attention regression and compile-fail matrix.

Do not split pure reduction, typestate, projection plans, backend mechanicalization, stable lookup,
compare-back, and semantic open into parallel or compatibility commits. Any intermediate state with
two semantic owners recreates the defect this commit exists to remove.

### Commit 3: `prove evm effect attention absorption`

Adapt only EVM behavior affected by the final interfaces and close acceptance:

- preserve `Ok(None) | Err(_) => EntryUnknown`;
- exercise manual, close-then-reassert, reassert, budget exhaustion, and restart paths through the
  public inventory and real Runtime drive;
- prove every closure uses the kernel discriminator and the exact preceding attempt;
- preserve the sealed entry-mode orphan rejection and the three access-kind equality guards;
- keep the two-process limitation and deployment sweep cadence honest;
- update `docs/known-gaps.md` and EVM-specific docs with the final absorption behavior, two-process
  limit, and deployment sweep cadence;
- run every deletion/ownership/review gate;
- delete this handoff only after every item passes, rerun gates without its exclusion, commit the
  final coherent tree, and run final CI once.

Commit 3 must not modify `crates/storages/evm-postgres/` relative to commit 2. Legitimate wallet
schema/identity work belongs in commit 1; Runtime-store schema work belongs in commit 2.

## Static deletion and ownership gates

Run these in strict Bash. While this handoff exists, exclude it. After completion, delete the file
and rerun without the exclusion. Mirror forbidden-family checks with `git grep` so force-tracked
ignored files cannot escape. Do not scan arbitrary ignored directories that may contain secrets;
derive and explicitly list repository-owned generated paths from `nixfied.nix`.

Every Bash fence from this heading through **Backend review gate** is one ordered script split only
for readability. Concatenate and execute the fences in the shown order in the same Bash process;
later blocks intentionally reuse `expect_no_matches`, `require_matches`, and the struct helpers.
The LOC and verification-section fences are separate self-contained commands.

```bash
set -euo pipefail

expect_no_matches() {
  local status
  if rg "$@"; then
    return 1
  else
    status=$?
    test "$status" -eq 1
  fi
}

expect_no_tracked_literal() {
  local literal=$1
  local status=0
  git grep -n -F -e "$literal" -- . \
    ':(exclude)TT1_SOLV_PROBLEM_10c53f2a.md' || status=$?
  if test "$status" -eq 0; then
    return 1
  fi
  test "$status" -eq 1
}

test ! -e crates/kernel/store/src/structured/fold.rs
test ! -e crates/kernel/store/src/structured/canonical_append.rs
test ! -e crates/kernel/store/src/structured/mutation.rs
test ! -e crates/kernel/store/src/structured/assembly.rs
test ! -e crates/kernel/store/src/structured/projection.rs
test ! -e crates/kernel/facts/src/codec.rs
test ! -e docs/effect-entry-resolution.md
test ! -e crates/kernel/store/tests/ui/fail/program_qualifier_is_not_a_public_trait.rs
test ! -e crates/kernel/store/tests/ui/fail/program_qualifier_is_not_a_public_trait.stderr
test ! -e contracts/recoverability
test ! -e crates/kernel/canonical/src/recoverability.rs
test ! -e crates/kernel/canonical/src/recoverability_limits.rs
test ! -e crates/kernel/canonical/tests/recoverability_v1.rs
test ! -e docs/recoverability-app-surface-v1.md
test ! -e docs/recoverability-predicate-owners-v1.md
test ! -e docs/recoverability-removal.md
test "$(find crates/storages/postgres/migrations -maxdepth 1 -type f | sort)" = \
  crates/storages/postgres/migrations/0001_store.sql
test -f crates/kernel/store/src/structured/validated_append.rs
test -f crates/kernel/store/src/structured/semantic_open.rs
test -f crates/kernel/store/tests/ui/fail/validated_append_cannot_be_counterfeited.rs
test -f crates/kernel/store/tests/ui/fail/validated_append_cannot_be_counterfeited.stderr
test -f crates/kernel/store/tests/ui/fail/program_trust_requires_the_certification_registry.rs
test -f crates/kernel/store/tests/ui/fail/program_trust_requires_the_certification_registry.stderr

expect_no_matches -n \
  '\bassemble_with_backend\b|\bassemble_structured_runtime\b|fn current_head\b' \
  crates/kernel/store crates/storages/postgres \
  --hidden --glob '!.git/**' --glob '!target/**'

expect_no_matches -n \
  'fold_recorded_history|VerifiedFoldState|\bFoldMachine\b|\bDerivationEngine\b|\bDerivedProgram\b|\bRecordedTransition\b|\bRecordedAuthorization\b|\bRecordedObservation\b|HistoryObjectOverlay|PhysicalObligationScope|\bProgramVerifier\b|\bRegistryProgramVerifier\b|ProgramVerifierSeal|VerifiedProgramData|build_program_verifier|\bQualifiedProgram\b|\bProgramQualifier\b|\bPendingReduction\b|PhysicalBindingVerification|allow_generate|generated_objects|placeholder_semantic_digest|placeholder_record_ref|mfm[.]structured-placeholder-access-attempt[.]v1|mfm[.]portable-export[.]digest-placeholder|verify_actionable_history|framework_schema_id' \
  . --hidden --glob '!.git/**' --glob '!target/**' \
  --glob '!TT1_SOLV_PROBLEM_10c53f2a.md'

# Exact journal-owned preimages may retain honest owner names. These duplicate store/certification
# preimages must not.
expect_no_matches -n \
  'AccessAttemptPreimage|ExpectedAccessAttemptPreimage|ExpectedRecordHashPreimage|AssignedRecordHashPreimage|AssignedCommitDigestPreimage|BorrowedCommitCandidate' \
  crates/kernel/store/src/structured/reducer.rs \
  crates/kernel/store/src/structured/coordinator.rs \
  crates/kernel/certify/src/structured.rs

expect_no_matches -n \
  'PublicPhysicalBindingVerifier|RetainedHistory|CurrentCandidate|ObservationQualification|ObservationEventQualification|qualify_observation|qualify_invoked_observation|PendingObservation|CommittedObservation|ObservationRetryBackoff|InvalidSupersessionEvidence|\bObservationCommit\b|\bAppendAttemptApi\b|\bAuthorizationApi\b|retain_verified|resolve_attempt|resolve_append|CanonicalRunAppend|CanonicalConfigurationAppend|RunProjectionMutation|StructuredAdmissionRequest|assign_candidate_for_test|verified_runs_equivalent|store_verified|is_store_verified' \
  . --hidden --glob '!.git/**' --glob '!target/**' \
  --glob '!TT1_SOLV_PROBLEM_10c53f2a.md'

expect_no_matches -n \
  'commit_prepared|prepare_admission|prepare_authorization|prepare_observation|prepare_state_transition|RunReducer::(start|prepare|finalize)' \
  . --hidden --glob '!.git/**' --glob '!target/**' \
  --glob '!TT1_SOLV_PROBLEM_10c53f2a.md'

expect_no_matches -n \
  'prepare_validated_genesis|extend_verified_candidate|reduce_(admission|transition|authorization|observation)_event|validate_transition_fact_coordinate|authorization_requires_fact_selection_barrier|validate_authorization_fact_coordinate|qualify_observation_event|proposed_observation|\bassign_candidate\b|AssignedRecordHashPreimage|AssignedCommitDigestPreimage|BorrowedCommitCandidate' \
  crates/kernel/store/src/structured/coordinator.rs

expect_no_matches -n \
  'ReducedEvent|ReducedObservationEvent|RecordedTransition|RecordedAuthorization|RecordedObservation|HistoryObjectLookup|HistoryObjectOverlay|validate_program_value_schemas' \
  crates/kernel/store/src/structured/qualification.rs \
  crates/kernel/store/src/structured/reducer.rs

expect_no_matches -n \
  'state_outcome_schema_id|operation_outcome_schema_id|fact_claim_schema_id|proposed_typed_object|\bframework_object\b|framework_schemas|StateOutcomeObject|OperationOutcomeObject|FactClaimObject|StructuredFactClaimPreimage|derive_frontier|state_frontier|fan_out_frontier|effect_entry_subject|minimum_action' \
  crates/kernel/store/src/structured/compiler.rs \
  crates/kernel/store/src/structured/reducer.rs

expect_no_matches -n \
  'into_runtime_attempt_with_successor|into_committed_successor|\bfact_scan_permit\b|[.]split[[:space:]]*\([[:space:]]*\)' \
  crates/kernel/store crates/storages/postgres \
  --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

expect_no_matches -n '\bfn split[[:space:]]*\(' \
  crates/kernel/store/src/structured/backend.rs \
  crates/kernel/store/src/structured/configuration.rs

expect_no_matches -n \
  'SCHEMA_SEED|mfm[.]structured-schema[.]v1|structured_content_ref|typed_content_ref|exact_content_ref|component_kind_schema_id|canonical_document_schema_id|certification_document_schema_id|CERTIFICATION_DOCUMENT_SCHEMAS|wallet_descriptor_ref|framework_object|fixed_schema_id|domain_content_digest|policy_expansion_recipe_ref' \
  crates --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

expect_no_matches -n \
  'RunProjectionReceipt|ProjectionReceipt|projection_receipt|receipt_digest|receipt_bytes|projection_contract_id' \
  . --hidden --glob '!.git/**' --glob '!target/**' \
  --glob '!TT1_SOLV_PROBLEM_10c53f2a.md'

expect_no_matches -n \
  'canonical_request_base64url|canonical_request_json|selection_schema_id|fact_query_digest_over_canonical_request|ComponentObjectEvidenceContract|PriorRunFactSelectorContract|FactCanonicalValue|PORTABLE_FRAME_SCHEMA_CONTRACT|PORTABLE_EXPORT_SCHEMA_CONTRACT|PortableFrameKind|PortableBatchPayload|placeholder_digest|previous_frame_digest|final_frame_digest|frame_chain_digest|genesis_chain_digest|chain_step|portable_chain_step' \
  . --hidden --glob '!.git/**' --glob '!target/**' \
  --glob '!TT1_SOLV_PROBLEM_10c53f2a.md'

expect_no_matches -n \
  'COMPONENT_OBJECT_EVIDENCE_CONTRACT_BYTES|RetainedValueContract::strict_decode' \
  crates/kernel/values crates/kernel/program-derive \
  --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

expect_no_matches -n \
  'std::thread|thread::Builder|\.stack_size[[:space:]]*\(|\.spawn[[:space:]]*\(' \
  crates/kernel/program-derive/src/lib.rs

expect_no_matches -U --pcre2 -n \
  '(?s)fn[[:space:]]+validate[[:space:]]*\(&self\)[^{]*\{[[:space:]]*(?:::std::result::Result::)?Ok\(\(\)\)[[:space:]]*\}' \
  crates/kernel/program-derive/src/lib.rs

expect_no_matches -n \
  'fact_selection_request_schema_id|fact_selection_query_schema_id|SELECTOR_CONTRACT_BYTES|from_canonical_value|canonical_request_base64url|canonical_request_json' \
  crates/kernel/facts/src/selection.rs

expect_no_matches -n \
  'FactCanonicalValue|PlainCanonicalJsonBytes|parse_canonical|\bfn lift\b' \
  crates/kernel/facts/src/value.rs

expect_no_matches -n \
  'prior_run_fact_source_manifest_schema_id|prior_run_fact_scanner_binding_schema_id|fn derive_(candidate_digest|access_attempt_id|record_hash|commit_digest)[[:space:]]*<|fn persisted_identity[[:space:]]*\(|\bDomainEnvelope\b|domain_envelope_digest' \
  crates/kernel/journal/src --glob '*.rs'

expect_no_matches -n \
  'capability_expansion_requirement_schema_id|contract_object_type|validate_component_object' \
  crates/kernel/spec/src crates/kernel/certify/src --glob '*.rs'

expect_no_matches -n \
  'fn (validated|decode)[[:space:]]*<|component_schema_id|fn schema_name[[:space:]]*\(' \
  crates/kernel/spec/src/public.rs crates/kernel/spec/src/structured.rs

expect_no_matches -n \
  'fn (component_object|decode_component)[[:space:]]*<' \
  crates/kernel/certify/src/structured.rs

expect_no_matches -n \
  'to_history_object|from_history_object' \
  crates --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

expect_no_matches -n \
  'serde_json::Value|serde_json::(to_value|from_value)|\bPortableFrame\b|PortableFrameKind|PortableBatchPayload|schema_identity|placeholder_digest|previous_frame_digest|final_frame_digest|frame_chain_digest|genesis_chain_digest|portable_chain_step|\bclosure_reference\b|authorized_closure_reference|compute_closure_reference|\bframe_count\b|\brecord_count\b|\bbyte_count\b|^[[:space:]]*ordinal[[:space:]]*:|seal_size|PortableRunExportV2|mfm[.]portable[^"[:space:]]*[.]v2' \
  crates/kernel/replay/src/portable.rs

expect_no_matches -n -i \
  'PortableRunExportV2|mfm[.]portable[^"[:space:]]*[.]v2|portable[-_ ]v2' \
  crates/kernel/replay tests docs --hidden --glob '!.git/**' --glob '!target/**'

expect_no_matches -n \
  'evidence_reference|mfm[.]evm[.]wallet-storage-evidence|reservation_evidence_ref|activation_evidence_ref|completion_evidence_ref|winning_activation_evidence_ref|predecessor_activation_ref|observed_floor_ref|original_terminal_witnesses_ref' \
  . --hidden --glob '!.git/**' --glob '!target/**' \
  --glob '!TT1_SOLV_PROBLEM_10c53f2a.md'

expect_no_matches -n \
  'mfm[.]evm[.]wallet-authority-postgres[.]v1' \
  crates/storages/evm-postgres/src \
  crates/storages/evm-postgres/migrations
expect_no_matches -n \
  'const PROTOCOL_VERSION:[[:space:]]*u16[[:space:]]*=[[:space:]]*3;' \
  crates/storages/evm-postgres/src/provider.rs \
  tests/wallet-authority-provider/src/lib.rs

expect_no_matches -n -i \
  'sole callback-free fold|complete records and append heads in physical chronology' \
  README.md crates docs --hidden --glob '!.git/**' --glob '!target/**' \
  --glob '!TT1_SOLV_PROBLEM_10c53f2a.md'

expect_no_matches -n \
  'RecoverabilityContract|RecoverabilityErrorCode|CanonicalReferencePath|recoverability-postgres-v1|recoverability-app-surface-v1|recoverability-predicate-owners-v1' \
  . --hidden --glob '!.git/**' --glob '!target/**' \
  --glob '!TT1_SOLV_PROBLEM_10c53f2a.md'

expect_no_matches -n \
  'HistoryObject::new' \
  crates --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

expect_no_matches -U -n \
  'pub fn new[[:space:]]*\([[:space:]]*object_type:[[:space:]]*StableId|pub fn decode[[:space:]]*<' \
  crates/kernel/journal/src/structured.rs

expect_no_matches -n \
  '\bfn (strict_decode|canonical_json|validated)\b|impl.*Deserialize.*RetainedValueContract' \
  crates/kernel/values/src/retained.rs

expect_no_matches -n \
  'media_type[[:space:]]*:[[:space:]]*(String|impl[[:space:]]+Into<String>|&[^,}]*str)' \
  crates/kernel/values/src/retained.rs crates/kernel/facts/src/emission.rs

expect_no_matches -n \
  'fn hash[[:space:]]*<[^>]*Serialize[^>]*>[[:space:]]*\(' \
  crates/domains/evm/src/wallet_authority.rs

expect_no_matches -n \
  'wallet_policy_schemas|fn (identity|text|flag|count|text_set)[[:space:]]*\(' \
  crates/domains/evm/src/wallet.rs

expect_no_matches -n \
  '\bCurrentOnly\b' \
  crates/kernel/store/src/structured/obligations.rs \
  crates/kernel/store/src/structured/coordinator.rs

expect_no_matches -n \
  'CertifiedProgramAuditRefs|validate_admission_audit_refs|certified_program_root_ref' \
  . --hidden --glob '!.git/**' --glob '!target/**' \
  --glob '!TT1_SOLV_PROBLEM_10c53f2a.md'

expect_no_matches -n \
  'ClosedSumContractPreimage|StructuredFactDescriptorPreimage|StructuredStateContractPreimage|ExpansionPolicyContractPreimage|QualifiedPolicyPreimage|derived_admission_policy_ref|fixture_component_with_same_schema|TestHistoryPayload' \
  . --hidden --glob '!.git/**' --glob '!target/**' \
  --glob '!TT1_SOLV_PROBLEM_10c53f2a.md'

expect_no_matches -n \
  'RevisionPreimage|\brevision_ref\b|\bvalue_contract_schema_id\b|\bvalue_contract_digest\b|\bvalue_schema_id\b|\bvalue_digest\b' \
  crates/kernel/store/src/structured/configuration.rs \
  crates/app/src/production_structured.rs \
  crates/storages/postgres/src/configuration.rs \
  crates/storages/postgres/src/schema.rs \
  crates/storages/postgres/migrations/0001_store.sql

expect_no_matches -n \
  'MFM_EMIT_CATALOG_MANIFEST|CATALOG-MANIFEST|emit_catalog_manifest_hashes_for_the_current_baseline|catalog_manifest_hashes_for_maintenance' \
  crates/storages/postgres crates/storages/evm-postgres \
  --hidden --glob '!.git/**' --glob '!target/**'

expect_no_matches -n \
  'EFFECT_ENTRY_ATTENTION_CURSOR_VERSION|\bRunAccessGrant\b|\bRunAccessPolicy\b|EffectEntryAttentionEvidence|EffectEntryAttentionPageEvidence' \
  crates tests bin docs --hidden --glob '!.git/**' --glob '!target/**'

expect_no_matches -n \
  'PendingTenantFactPublication|TenantFactRouteSummary|publication_count|minimum_order|maximum_order|validate_prefix_integrity|ordered_configuration|invalid_configuration|verify_configuration_history|\braw_head\b|\bfull_loads\b|ExternalCheckpointAuthority' \
  crates/storages/postgres \
  crates/kernel/store/src/structured/memory.rs \
  --hidden --glob '!.git/**' --glob '!target/**'

expect_no_matches -n \
  'TenantFactCoordinate::|RunRecord::|\bAppendKey\b|^[[:space:]]*appends:' \
  crates/storages/postgres/src \
  crates/kernel/store/src/structured/memory.rs \
  --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

expect_no_matches -n \
  'run_access_attempts|run_access_attempts_possible_entry' \
  crates/storages/postgres crates/kernel/store \
  --hidden --glob '!.git/**' --glob '!target/**'

expect_no_matches -n \
  'HistoryObjectPayload' \
  crates --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

tracked_forbidden_literals=(
  fold_recorded_history
  FoldMachine
  VerifiedFoldState
  DerivationEngine
  DerivedProgram
  RecordedTransition
  RecordedAuthorization
  RecordedObservation
  HistoryObjectOverlay
  PhysicalObligationScope
  ProgramVerifier
  ProgramVerifierSeal
  RegistryProgramVerifier
  VerifiedProgramData
  build_program_verifier
  QualifiedProgram
  ProgramQualifier
  PendingReduction
  PhysicalBindingVerification
  allow_generate
  generated_objects
  placeholder_semantic_digest
  placeholder_record_ref
  verify_actionable_history
  framework_schema_id
  PublicPhysicalBindingVerifier
  RetainedHistory
  CurrentCandidate
  ObservationQualification
  ObservationEventQualification
  qualify_observation
  qualify_invoked_observation
  PendingObservation
  CommittedObservation
  ObservationRetryBackoff
  InvalidSupersessionEvidence
  ObservationCommit
  AppendAttemptApi
  AuthorizationApi
  retain_verified
  resolve_attempt
  resolve_append
  CanonicalRunAppend
  CanonicalConfigurationAppend
  RunProjectionMutation
  StructuredAdmissionRequest
  assign_candidate_for_test
  verified_runs_equivalent
  store_verified
  is_store_verified
  commit_prepared
  prepare_admission
  prepare_authorization
  prepare_observation
  prepare_state_transition
  prepare_validated_genesis
  extend_verified_candidate
  reduce_admission_event
  reduce_transition_event
  reduce_authorization_event
  reduce_observation_event
  validate_transition_fact_coordinate
  authorization_requires_fact_selection_barrier
  validate_authorization_fact_coordinate
  proposed_observation
  into_runtime_attempt_with_successor
  into_committed_successor
  fact_scan_permit
  AssignedRecordHashPreimage
  AssignedCommitDigestPreimage
  BorrowedCommitCandidate
  SCHEMA_SEED
  mfm.structured-schema.v1
  exact_content_ref
  component_kind_schema_id
  canonical_document_schema_id
  certification_document_schema_id
  CERTIFICATION_DOCUMENT_SCHEMAS
  capability_expansion_requirement_schema_id
  contract_object_type
  validate_component_object
  wallet_descriptor_ref
  wallet_policy_schemas
  framework_object
  framework_schemas
  StateOutcomeObject
  OperationOutcomeObject
  FactClaimObject
  StructuredFactClaimPreimage
  fixed_schema_id
  policy_expansion_recipe_ref
  RunProjectionReceipt
  ProjectionReceipt
  projection_receipt
  receipt_digest
  receipt_bytes
  projection_contract_id
  structured_content_ref
  typed_content_ref
  domain_content_digest
  persisted_identity
  DomainEnvelope
  domain_envelope_digest
  selection_schema_id
  canonical_request_base64url
  canonical_request_json
  fact_query_digest_over_canonical_request
  ComponentObjectEvidenceContract
  PriorRunFactSelectorContract
  FactCanonicalValue
  PORTABLE_FRAME_SCHEMA_CONTRACT
  PORTABLE_EXPORT_SCHEMA_CONTRACT
  PortableFrameKind
  PortableBatchPayload
  placeholder_digest
  previous_frame_digest
  final_frame_digest
  frame_chain_digest
  genesis_chain_digest
  portable_chain_step
  CertifiedProgramAuditRefs
  validate_admission_audit_refs
  certified_program_root_ref
  ClosedSumContractPreimage
  StructuredFactDescriptorPreimage
  StructuredStateContractPreimage
  ExpansionPolicyContractPreimage
  QualifiedPolicyPreimage
  derived_admission_policy_ref
  fixture_component_with_same_schema
  to_history_object
  from_history_object
  TestHistoryPayload
  HistoryObjectPayload
  HistoryObject::new
  RevisionPreimage
  COMPONENT_OBJECT_EVIDENCE_CONTRACT_BYTES
  SELECTOR_CONTRACT_BYTES
  fact_selection_request_schema_id
  fact_selection_query_schema_id
  PendingTenantFactPublication
  TenantFactRouteSummary
  publication_count
  minimum_order
  maximum_order
  validate_prefix_integrity
  ordered_configuration
  invalid_configuration
  raw_head
  full_loads
  MFM_EMIT_CATALOG_MANIFEST
  CATALOG-MANIFEST
  emit_catalog_manifest_hashes_for_the_current_baseline
  catalog_manifest_hashes_for_maintenance
  run_access_attempts
  RunAccessGrant
  RunAccessPolicy
  EFFECT_ENTRY_ATTENTION_CURSOR_VERSION
  EffectEntryAttentionEvidence
  EffectEntryAttentionPageEvidence
  ExternalCheckpointAuthority
  evidence_reference
  mfm.evm.wallet-storage-evidence
  reservation_evidence_ref
  activation_evidence_ref
  completion_evidence_ref
  winning_activation_evidence_ref
  predecessor_activation_ref
  observed_floor_ref
  original_terminal_witnesses_ref
  'sole callback-free fold'
  'complete records and append heads in physical chronology'
  RecoverabilityContract
  RecoverabilityErrorCode
  CanonicalReferencePath
  recoverability-postgres-v1
  recoverability-app-surface-v1
  recoverability-predicate-owners-v1
)
for literal in "${tracked_forbidden_literals[@]}"; do
  expect_no_tracked_literal "$literal"
done

# No repository-owned SQLx metadata directory exists at the audited tree. If final tooling creates
# either conventional crate-local directory, scan only that known-safe generated root.
for generated_root in \
  crates/storages/postgres/.sqlx \
  crates/storages/evm-postgres/.sqlx
do
  test ! -d "$generated_root" && continue
  for literal in "${tracked_forbidden_literals[@]}"; do
    expect_no_matches --no-ignore -n -F "$literal" "$generated_root"
  done
done

extract_struct_block() {
  local path=$1 type_name=$2
  awk -v type_name="$type_name" '
    $0 ~ "(^|[[:space:]])struct[[:space:]]+" type_name "[[:space:]]*\\{" { found = 1 }
    found { print }
    found && /^[[:space:]]*}/ { exit }
  ' "$path"
}

assert_struct_lacks_field() {
  local path=$1 type_name=$2 field_name=$3 block
  block=$(extract_struct_block "$path" "$type_name")
  test -n "$block"
  if printf '%s\n' "$block" | rg "^[[:space:]]*(pub([[:space:]]|\\([^)]*\\)[[:space:]]+)?)?${field_name}[[:space:]]*:"; then
    exit 1
  fi
}

assert_struct_lacks_field crates/kernel/spec/src/structured.rs ClosedSumContract closed_sum_contract_ref
assert_struct_lacks_field crates/kernel/spec/src/structured.rs StructuredFactDescriptor descriptor_ref
assert_struct_lacks_field crates/kernel/spec/src/structured.rs StructuredStateContract state_contract_ref
assert_struct_lacks_field crates/kernel/spec/src/structured.rs ExpansionPolicyContract policy_ref
assert_struct_lacks_field \
  crates/kernel/certify/src/structured.rs \
  QualifiedStructuredEntryPointPolicy \
  admission_policy_ref
assert_struct_lacks_field crates/kernel/store/src/structured/configuration.rs ConfigurationRevision revision_ref

for owner_spec in \
  'crates/kernel/values/src/retained.rs:RetainedValueContract' \
  'crates/kernel/facts/src/emission.rs:ProposedFactValue' \
  'crates/kernel/facts/src/emission.rs:ProposedFactValueIdentity'
do
  owner_path=${owner_spec%%:*}
  owner_type=${owner_spec##*:}
  owner_block=$(extract_struct_block "$owner_path" "$owner_type")
  test -n "$owner_block"
  printf '%s\n' "$owner_block" | rg \
    '^[[:space:]]*(pub[^:]*)?media_type[[:space:]]*:[[:space:]]*MediaType[[:space:]]*,'
done

configuration_head_block=$(
  extract_struct_block \
    crates/kernel/store/src/structured/configuration.rs \
    ConfigurationHistoryHead
)
printf '%s\n' "$configuration_head_block" | rg '^[[:space:]]*(pub[^:]*)?object_ref[[:space:]]*:'

mapfile -t snapshot_definition_paths < <(
  rg -l '\bstruct StructuredRunSnapshot\b' crates/kernel/store --glob '*.rs' | sort
)
test "${#snapshot_definition_paths[@]}" -eq 1
snapshot_block=$(extract_struct_block "${snapshot_definition_paths[0]}" StructuredRunSnapshot)
test -n "$snapshot_block"
if printf '%s\n' "$snapshot_block" | rg '^[[:space:]]*(pub[^:]*)?head[[:space:]]*:'; then
  exit 1
fi
printf '%s\n' "$snapshot_block" | rg '^[[:space:]]*(pub[^:]*)?current_projection[[:space:]]*:'

for owner_type in \
  StructuralPath \
  StructuredLiveComponentContract \
  SecretFreeExecutableIdentity \
  SecretFreeQualificationArtifact \
  SecretFreeImplementationDescriptor \
  AuthoredStructuredProgram \
  ExpandedStructuredProgram \
  ExpandedLexicalSlot \
  StructuredExpansionProfile \
  CertifiedProgramRoot \
  CertifiedProgramDocument \
  PlanningProfile
do
  expect_no_matches -U --pcre2 -n \
    "(?ms)^impl[[:space:]]+${owner_type}[[:space:]]*\\{(?:(?!^\\}).)*\\bfn[[:space:]]+content_ref\\b" \
    crates/kernel/spec/src/structured.rs crates/kernel/spec/src/public.rs
done
```

The final reducer boundary must be empty for:

```bash
mapfile -t reducer_closure_files < <(
  rg --files crates/kernel/store/src/structured --glob '*.rs' |
    rg '/reducer([_/]|[.]rs$)' |
    sort
)
test "${#reducer_closure_files[@]}" -eq 1
test "${reducer_closure_files[0]}" = crates/kernel/store/src/structured/reducer.rs

expect_no_matches -n \
  'CommittedBatch|AssignedRecord|HistoryObject|RawRunHistory|RunRecord|RecordedAssertions|serde_json|\bdyn\b|\basync\b|\bawait\b|impl Future|Future<|\bFn(Once|Mut)?[[:space:]]*\(|Verifier|Backend|Postgres|sqlx|std::fs|tokio::fs|reqwest|Runtime.*Proposal|CanonicalRunAppend|ValidatedRunAppend|RunProjectionReceipt|FactScanPermitSpec|TenantFactProjectionPlan|allow_generate|generated_objects|PhysicalBindingVerificationMode|canonical_json|HistoryObjectLookup|decode_component|validate_program_value_schemas' \
  "${reducer_closure_files[@]}"

expect_no_matches -n \
  'StateLeaf::|StructuredFrontier::|ActionableState|walk_program|find_state|decode_component' \
  crates/kernel/store/src/structured/compiler.rs

expect_no_matches -n \
  'StateLeaf|StructuredFrontier|ActionableState|ReducedRunState|FoldMachine|walk_program|find_state' \
  crates/kernel/store/src/structured/qualification.rs
```

Inspect every reducer import, generic, argument, and return type. Grep cannot detect a renamed
statically dispatched callback.

Positive owner gates:

```bash
set -euo pipefail

require_matches() {
  local status=0
  rg "$@" || status=$?
  test "$status" -eq 0
}

require_exactly_one() {
  local matches
  local status=0
  matches=$(rg "$@") || status=$?
  test "$status" -eq 0
  test "$(printf '%s\n' "$matches" | wc -l)" -eq 1
  printf '%s\n' "$matches"
}

require_exactly_one -n \
  '^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?fn reduce_event\b' \
  crates/kernel/store/src/structured/reducer.rs

require_exactly_one -n '\bstruct PendingSemanticStep\b' \
  crates/kernel/store/src/structured/reducer.rs
require_exactly_one -n '\bstruct ComparedReduction\b' \
  crates/kernel/store/src/structured/compiler.rs
require_exactly_one -n '\bstruct FinalizedReduction\b' \
  crates/kernel/store/src/structured/obligations.rs
require_exactly_one -n '\bstruct VerifiedStructuredRun\b' \
  crates/kernel/store/src/structured/mod.rs
require_exactly_one -n '\bfn from_finalized\b' \
  crates/kernel/store/src/structured/mod.rs
require_exactly_one -n \
  '^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?(async[[:space:]]+)?fn commit_event\b' \
  crates/kernel/store/src/structured/coordinator.rs
require_exactly_one -n '\bfn mint_committed_fact_read_capability\b' \
  crates/kernel/store/src/structured/coordinator.rs
require_exactly_one -n \
  '^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?async[[:space:]]+fn qualify_and_open_structured_store\b' \
  crates/kernel/store/src/structured/semantic_open.rs
require_exactly_one -n '\btrait PersistedObjectPayload\b' crates/kernel/values/src

for type_name in \
  EvmWalletNoncePolicyDocument \
  EvmWalletAssurancePolicyDocument \
  EvmDeterministicSigningProfileDocument \
  EvmSubmissionExpansionPolicyDocument
do
  require_exactly_one -n "^[[:space:]]*struct ${type_name}[[:space:]]*\\{" \
    crates/domains/evm/src/wallet.rs
done

require_matches -n '\bcandidate_operation_key\b' crates/domains/evm crates/storages/evm-postgres
require_matches -n '\bpredecessor_candidate_operation_key\b' \
  crates/domains/evm crates/storages/evm-postgres
require_matches -n 'mfm[.]evm[.]wallet-authority-postgres[.]v2' \
  crates/storages/evm-postgres/migrations/0001_wallet_authority.sql \
  crates/storages/evm-postgres/src/schema.rs
for provider_path in \
  crates/storages/evm-postgres/src/provider.rs \
  tests/wallet-authority-provider/src/lib.rs
do
  require_matches -n 'const PROTOCOL_VERSION:[[:space:]]*u16[[:space:]]*=[[:space:]]*4;' \
    "$provider_path"
done

for type_name in QualifiedRunContext QualifiedHistory QualifiedBatch RecordedAssertions; do
  require_exactly_one -n "\bstruct ${type_name}\b" \
    crates/kernel/store/src/structured/qualification.rs
done
for type_name in ReducedRunState UnboundSuccessor; do
  require_exactly_one -n "\bstruct ${type_name}\b" \
    crates/kernel/store/src/structured/reducer.rs
done
require_exactly_one -n '\bstruct FactScanPermitSpec\b' \
  crates/kernel/store/src/structured/compiler.rs
require_exactly_one -n '\b(enum|struct) TenantFactProjectionPlan\b' \
  crates/kernel/store/src/structured/validated_append.rs
require_exactly_one -n '\bstruct ConfigurationRevisionObject\b' \
  crates/kernel/store/src/structured/configuration.rs

require_matches -n '\bPersistedObjectPayload\b' crates/kernel/journal

for token in lookup_append_attempt append_request_id; do
  require_matches -n "\b${token}\b" crates/kernel/store crates/storages/postgres
done

require_matches -n '\bfn lookup_append_attempt\b' crates/kernel/store/src/structured/backend.rs
require_matches -n '\bfn lookup_append_attempt\b' crates/kernel/store/src/structured/memory.rs
require_matches -n '\bfn lookup_append_attempt\b' crates/storages/postgres/src/structured.rs

for backend_path in \
  crates/kernel/store/src/structured/backend.rs \
  crates/kernel/store/src/structured/memory.rs \
  crates/storages/postgres/src/structured.rs
do
  require_matches -U -n '(?s)fn load_prefix\b[^;{]{0,600}\bJournalHead\b' "$backend_path"
  require_matches -n '\bfn current_run_projection\b' "$backend_path"
done
expect_no_matches -n '\bthrough_sequence\b' \
  crates/kernel/store/src/structured/backend.rs \
  crates/kernel/store/src/structured/memory.rs \
  crates/storages/postgres/src/structured.rs

require_exactly_one -n '\bstruct ValidatedRunAppend\b' \
  crates/kernel/store/src/structured/validated_append.rs
require_exactly_one -n '\bstruct ValidatedConfigurationAppend\b' \
  crates/kernel/store/src/structured/validated_append.rs
require_exactly_one -n '\bstruct RunProjectionPlan\b' \
  crates/kernel/store/src/structured/validated_append.rs
require_exactly_one -n '\benum ApplicationAccessGrant\b' crates/app/src/access.rs
require_exactly_one -n '\btrait ApplicationAccessPolicy\b' crates/app/src/access.rs
require_exactly_one -n '\benum EffectEntryAttentionResolution\b' \
  crates/kernel/runtime/src/history/cursor.rs
require_exactly_one -n '\benum EffectEntryAttentionResolution\b' crates --glob '*.rs'
require_exactly_one -n '\bstruct EffectEntryAttention\b' \
  crates/kernel/store/src/structured/reducer.rs
require_exactly_one -n '\bstruct EffectEntryAttentionPage\b' \
  crates/kernel/store/src/structured/purpose.rs

mapfile -t verified_constructor_callers < <(
  rg -l 'VerifiedStructuredRun::from_finalized' \
    crates/kernel/store/src/structured --glob '*.rs' | sort
)
expected_verified_constructor_callers=(
  crates/kernel/store/src/structured/coordinator.rs
  crates/kernel/store/src/structured/semantic_open.rs
)
test "${verified_constructor_callers[*]}" = "${expected_verified_constructor_callers[*]}"

for token in CloseThenReassert Reassert has_effect_entry_attention; do
  require_matches -n "\b${token}\b" crates tests bin docs
done
```

Review those hits; names/counts alone did not protect `10c53f2a`.

The certified-policy ownership census must produce exactly the three approved paths listed earlier:

```bash
mapfile -t certified_policy_paths < <(
  rg -l '\bqualified_entry_point_admission_policy_ref\b' crates --glob '*.rs' | sort
)
expected_certified_policy_paths=(
  crates/kernel/certify/src/structured.rs
  crates/kernel/certify/tests/structured_certification.rs
  crates/kernel/spec/src/structured.rs
)
test "${certified_policy_paths[*]}" = "${expected_certified_policy_paths[*]}"
```

In addition, tests must prove the field occurs only on the certified-root component, not another
struct in one of those files.

The app attention structs must not regain a second serializer or writable data surface:

```bash
for type_name in EffectEntryAttentionEntry EffectEntryAttentionPage; do
  block=$(
    awk -v type_name="$type_name" '
      { source[NR] = $0 }
      $0 ~ "pub struct " type_name "[[:space:]]*\\{" { start = NR - 4; found = 1 }
      found && /^}/ {
        for (line = start; line <= NR; line++) print source[line]
        exit
      }
    ' crates/app/src/surface.rs
  )
  test -n "$block"
  if printf '%s\n' "$block" | rg '\b(Serialize|Deserialize)\b|#\[serde|^[[:space:]]+pub([[:space:]]|\()'; then
    exit 1
  fi
done

cursor_block=$(
  awk '
    /struct EffectEntryAttentionCursorWire[[:space:]]*\{/ { found = 1 }
    found { print }
    found && /^}/ { exit }
  ' crates/app/src/surface.rs
)
test -n "$cursor_block"
if printf '%s\n' "$cursor_block" | rg '\bversion\b'; then
  exit 1
fi
require_matches -n 'EFFECT_ENTRY_ATTENTION_CURSOR_PREFIX' crates/app/src/surface.rs
```

`app/render.rs` must still have the sole reviewed encoder for the attention page. Add a compile-time
`assert_not_impl_any!` test proving both attention surface types implement neither
`serde::Serialize` nor `serde::Deserialize`/`DeserializeOwned`; the source block check alone is not
the proof.

### Same-name and rehosted-code rejection review

Deletion gates cannot distinguish an honest replacement from the old body pasted behind an allowed
name. Before accepting commits 1 and 2, review the complete definitions—not selected call sites—and
record the file/line evidence for every applicable row:

| Final surface | Mandatory structural fingerprint | Automatic failure fingerprint |
| --- | --- | --- |
| `QualifiedEvent::Intent` | Carries one normalized typed Runtime intent | Carries `CommittedBatch`, assigned records, canonical bytes, or a pre-authored candidate |
| `QualifiedEvent::Recorded` | Carries only the normalized semantic event decoded from owner-qualified records; `RecordedAssertions` stays a separate compiler input | Carries assignment/hash/object metadata into the reducer, or performs object lookup, schema discovery, or serialization while reduction runs |
| `PendingSemanticStep` | Non-`Clone`; owns unbound successor, typed intents, fact requirement, and the complete obligation set | Wraps only the same reduced program/state as later typestates |
| `ComparedReduction` | Constructed only by independent compiler comparison of preview and qualified recorded products | Compares a batch/assertion/object with a value borrowed from that same batch/object |
| `FinalizedReduction` | Constructed only by a fallible, consuming whole-obligation discharge | `finish`, `new`, ignored scope, unconditional `Ok`, boolean verification, or partial/current-only discharge |
| `ValidatedRunAppend` | Constructed only by consuming finalized/compiler-owned products and contains both run and fact plans | Ordinary `pub(crate) new(batch, projection)`, batch-only extraction, or a constructor reachable from tests/adapters/backends |
| `StructuredRunSnapshot` | Exact immutable snapshot plus supplied current projection for compare-back | Duplicate `head`, `current_head`, or a head-only cache-validity answer |
| `lookup_append_attempt` | Mechanical immutable lookup used before any frontier read/candidate authoring and again under append locks | Candidate digest required before lookup or a separate answer cache/index |
| fact permit mint | One constructor call in the exact `NewlyCommitted` post-commit arm | Permit/spec stored in a prepared candidate, compiler, backend, `ExistingSame`, stale, conflict, or ambiguity arm |
| Effect attention | One reducer-owned `Option<EffectEntryAttention { subject, resolution }>` copied into projection and re-derived on load | `matches!(frontier, PossibleEntry(_))`, manual-only filtering, or app/backend/SQL derivation |
| journal semantic identities | Each operation accepts one exact named preimage/owner input and fixes its domain internally | Any generic `T: Serialize`, caller-supplied domain/name/shape, universal envelope, or “serializes identically” justification |
| journal persisted documents | Each concrete document implements the common persisted owner contract | Shared `persisted_identity(name, shape)`, string-selected schema, forwarding schema-ID helper, or caller-paired bytes/schema |
| EVM wallet policy documents | Four concrete typed documents own their fields, literals, bounds, canonical bytes, and identities | `serde_json::json!` paired with a separately assembled schema, generic field-descriptor builder, or shared wallet-policy envelope |
| fact selection request/query | One direct bounded typed representation and one owner-derived persisted identity | Nested canonical JSON/base64 document, cached reparsed bytes, name registry, generic field parser, or wrapper schema |
| portable v3 stream | One closed typed `Batch | Seal` record language and one complete-stream identity | Erased JSON payload, frame identity, chain/count digest, v2 path, or descriptor that describes only the seal |

Also inspect source lineage:

```bash
git diff --find-renames=40% --find-copies-harder --summary \
  a9ad9442fb0439b5334d3b107b9639d025ab1f58..HEAD
git diff --numstat a9ad9442fb0439b5334d3b107b9639d025ab1f58..HEAD -- \
  crates/kernel/store/src/structured \
  crates/kernel/runtime/src \
  crates/storages/postgres \
  crates/kernel/values/src \
  crates/kernel/facts/src \
  crates/kernel/journal/src \
  crates/kernel/spec/src \
  crates/kernel/certify/src \
  crates/kernel/replay/src/portable.rs \
  crates/domains/evm/src
```

Any rename/copy candidate that preserves a tombstoned responsibility fails even if the old symbol
is absent. The engineer must identify the final owner of each genuinely retained pure semantic rule;
“moved” is not a disposition for codecs, callbacks, factories, preflights, projection derivation,
backend semantics, or arbitrary identity helpers.

Backend review gate:

```bash
set -euo pipefail

review_rg() {
  local status=0
  rg "$@" || status=$?
  test "$status" -le 1
}

expect_no_matches -U -n \
  '\b(StructuredHistoryBackend|ConfigurationHistoryBackend)\s+as\s+[A-Za-z_][A-Za-z0-9_]*' \
  . --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

review_rg -n \
  'RunRecord|StateLeaf|StructuredFrontier|AccessKind|ObservationOutcome|TenantFactCoordinate|FactPublication|entry_budget|reassert|settlement' \
  crates/storages/postgres crates/kernel/store/src/structured/memory.rs \
  --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'

review_rg -n -i \
  'jsonb|json_extract|->>|#>>|#>|row_number|lead\(|lag\(|observed_outcome|access_kind|run_closed|fact_publication|configuration.*(head|revision)' \
  crates/storages/postgres/src crates/storages/postgres/migrations \
  --hidden --glob '!.git/**' --glob '!target/**'

review_rg -n \
  'fn (open_structured_authoritative|open_structured_authoritative_application|open_structured_authoritative_with_configuration|open_configuration_maintenance|assemble_structured_runtime)' \
  crates/storages/postgres crates/kernel/store \
  --hidden --glob '!.git/**' --glob '!target/**' --glob '*.rs'
```

Review the complete explicit implementation/source closure for every backend, including delegated
helpers and corrupt/injecting fixtures. Backend traits may not be aliased or macro-generated to
escape the census. Allowed hits are physical decoding, locks, CAS, immutable inserts, supplied
mechanical commands, and tests—not projection decisions.

The `tracked_forbidden_literals` loop above is the mandatory `git grep` mirror over tracked content;
the immediately following loop covers only the explicitly known safe SQLx metadata roots if they
appear. Do not replace either with a scan of arbitrary ignored directories.

The phrase `run parked` is not a valid deletion gate: conceptual parked-state prose is legitimate.
Gate obsolete public identifiers and false behavioral claims instead.

## Verification schedule

All Rust/Cargo tools run in the default Nix development shell. Use the narrowest target while
iterating and expand by affected boundary. Do not use host Rust.

### Persisted-identity commit

Select affected packages from the final census; the minimum expected set is:

```bash
nix develop -c cargo test -p mfm-canonical
nix develop -c cargo test -p mfm-values
nix develop -c cargo test -p mfm-facts
nix develop -c cargo test -p mfm-journal
nix develop -c cargo test -p mfm-spec
nix develop -c cargo test -p mfm-certify
nix develop -c cargo test -p mfm-signing
nix develop -c cargo test -p mfm-replay
nix develop -c cargo test -p mfm-evm
nix develop -c cargo test -p mfm-storage-evm-postgres --test provider-protocol
nix develop -c cargo test -p mfm-integration-tests --test replay_wire_contract
nix run .#run -- --task cargo-metadata-contract
```

### Runtime/store iteration

```bash
nix develop -c cargo test -p mfm-runtime --test structured-runtime
nix develop -c cargo test -p mfm-store --lib
nix develop -c cargo test -p mfm-store --test api-surface
nix develop -c cargo test -p mfm-store --test structured-history-qualification
nix develop -c cargo test -p mfm-store --test structured-runtime
nix develop -c cargo test -p mfm-store --test structured-runtime-causal
nix develop -c cargo test -p mfm-replay
nix develop -c cargo test -p mfm-app
nix develop -c cargo test -p mfm
nix develop -c cargo test -p mfm-rest-api
```

Add dedicated reducer-equivalence, backend-conformance, projection-corruption, and attention test
targets if putting the matrix into existing files would make them unwieldy. Run whole named targets
so a missing target is an error.

### Database and model iteration

Confirm task IDs in `nixfied.nix`; at the time of this handoff they are:

```bash
nix run .#model-check
nix run .#run -- --task cargo-metadata-contract
nix run .#run -- --task postgres-sql-inventory-check
nix run .#run -- --task postgres-sqlx-offline-check
nix run .#run -- --task postgres-sqlx-check
nix run .#run -- --task structured-history-postgres-qualification
nix run .#run -- --task wallet-nonce-postgres-storage-qualification
nix run .#run -- --task evm-postgres-submission-qualification
```

`postgres-sqlx-check` is mandatory after the final migrations stabilize. It creates fresh schemas,
runs migrations, verifies both catalog manifests, checks SQLx, and exercises the hostile catalog-
mutation probe. SQL inventory and offline SQLx checks cannot substitute for it.

### Final acceptance

Before final CI:

1. Complete all owner-focused tests, behavioral matrices, corruption tests, compile-fail tests,
   deletion gates, and human authority reviews.
2. Regenerate catalog attestations only after migrations are final; remove temporary emitters.
3. Run:

   ```bash
   nix develop -c cargo fmt --all -- --check
   git diff --check
   ```

4. Delete `TT1_SOLV_PROBLEM_10c53f2a.md` and rerun all `rg` and `git grep` gates without exclusion.
5. Commit the final coherent tree. With the verified pre-rewrite working commit temporarily tagged
   `correction-working-final`, require exact tree and topology equality:

   ```bash
   cutover_parent=56c260ba492ef5de71e3ef82250295cd583a2868
   test "$(git rev-parse HEAD^^^^)" = "$cutover_parent"
   test "$(git rev-parse correction-working-final^{tree})" = "$(git rev-parse HEAD^{tree})"

   expected_subjects=$'plan the 10c53f2a runtime-store correction\nfinish rust-owned persisted identity ownership\nreplace runtime storage with three qualified layers\nprove evm effect attention absorption'
   actual_subjects=$(git log --reverse --format=%s "$cutover_parent"..HEAD)
   test "$actual_subjects" = "$expected_subjects"

   status=0
   git merge-base --is-ancestor a9ad9442fb0439b5334d3b107b9639d025ab1f58 HEAD || status=$?
   test "$status" -eq 1
   ```

   Delete the temporary `correction-working-final` tag and all temporary correction branches after
   these checks; preserve only the private pre-cutover backup ref. Then require:

   ```bash
   test -z "$(git status --porcelain)"
   test "$(git rev-parse origin/refact-runtime)" = \
     35fd0144a6daf9c06e33087751b1880aced87e9b
   git log --oneline origin/refact-runtime..HEAD
   git rev-parse HEAD
   ```

   Record the SHA. Since commit 3 is the final commit and commit 2 is its direct parent, also prove
   the EVM PostgreSQL storage tree was untouched by commit 3:

   ```bash
   git diff --exit-code HEAD^..HEAD -- crates/storages/evm-postgres/
   ```
6. Run exactly once on that final SHA:

   ```bash
   nix run .#ci
   ```

Do not immediately run `.#check`, `.#test`, and `.#test-db` before `.#ci`; CI already composes them.
Record exact Nixfied evidence and any remaining unverified external limitation.

## Final acceptance checklist

Delete this handoff only when every statement is true:

- the recorded delete-first checkpoint removed at least 5,031 runtime/store implementation lines
  with zero additions before replacements were written;
- commit 1 has at least 1,100 credited persisted-identity production deletions after the conservative
  inline-test deduction and is net-negative over the exact owner manifest;
- semantic core, complete structured-store source, facts source, and PostgreSQL source/baseline are
  each net-negative from `a9ad9442f` and at or below 5,341, 11,058, 1,782, and 6,668 lines
  respectively;
- persisted identities are derived from exact Rust owner shapes; hostile container bounds are
  enforced at canonical ingress;
- journal identity operations use fixed owner-local domains and exact named preimages, with no
  generic persisted-identity factory or universal domain envelope;
- the four EVM wallet policy documents are concrete typed owners, with no generic name/field schema
  builder or separately assembled JSON/schema pair;
- fact selection has one direct typed request/query representation;
- production `HistoryObject` construction/decoding is typed by its owner;
- no caller-selected semantic-domain hash helper or production generic schema/object factory
  remains;
- configuration has one payload and one content reference across store and admission;
- portable v3 has one closed JSON-lines stream identity and no erased frame identity;
- certified admission policy is present only in the certified-root owner closure, while
  `RunAdmitted` carries only `certified_program_ref`;
- `VerifiedStructuredRun` is exactly qualified immutable evidence plus compact reduced state;
- `reduce_event` is the sole synchronous deterministic transition rule and has no callback,
  verifier, IO, authoring, serialization, persistence, or raw-record dependency;
- intent preview, requalified recorded candidate, full replay, and incremental advancement execute
  that same rule and compare equal at every prefix;
- `PendingSemanticStep -> ComparedReduction -> FinalizedReduction` is a real consuming proof chain;
- physical/currentness and prior-run checks are closed obligations outside the reducer;
- one compiler owns records, artifacts, assignment, assertion comparison, projection plans, and
  `FactScanPermitSpec`;
- only the post-`NewlyCommitted` coordinator mints an affine fact permit;
- backends accept sealed commands and perform only decoding, stable lookup, locks, exact compare,
  immutable insert, CAS, and supplied-plan application;
- stable `(run_id, append_request_id)` lookup precedes every frontier-dependent authoring decision;
- current and historical retries cannot rewind projections, cache historical successors, or mint
  permits;
- every ordinary load/cache use compares the complete current projection;
- semantic open audits all authoritative/projection keys at pinned `S0`, compares every projection
  in both directions, and exposes no handle before final fence/release revalidation;
- normal open fails closed and never repairs;
- Effect attention is orthogonal to frontier and returns `Manual`, `CloseThenReassert`, or
  `Reassert`; Read never enters it;
- PostgreSQL retains one current per-run attention index and no historical attempt route;
- tenant fact heads/routes and configuration heads are minimal, mechanical, and globally
  compare-backed;
- observation/supersession preflight, mixed fold state, old canonical append file, generic identity
  factories, receipt concepts, `mutation.rs`, `assembly.rs`, `projection.rs`, facts `codec.rs`,
  `docs/effect-entry-resolution.md`, and stale documentation are deleted;
- the complete regression matrix passes against memory, PostgreSQL, replay, public surfaces, and
  EVM;
- EVM preserves unknown-on-absence, kernel closure discrimination, budget-bounded reassertion, and
  honest two-process/sweep limitations;
- focused owner, metadata, model, SQL inventory, SQLx, live schema, store qualification, wallet
  qualification, and EVM submission evidence is recorded;
- this file is deleted, post-deletion gates are clean, the final tree is committed and clean, and
  one final `nix run .#ci` passes on the recorded SHA;
- the rewritten branch remains unpushed and the private backup ref is the only preservation of the
  superseded local history.

The resulting repository—not a retained plan, comment, compatibility path, or passing symbol-count
gate—is the proof of the current design.
