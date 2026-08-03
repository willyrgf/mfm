# TT1 Runtime Remediation Review

Independent review evidence for `IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md` remediation
checkpoints. Each checkpoint is append-only and records an explicit PASS/FAIL gate.

---

## Kernel checkpoint (after Commit 3)

### Identity

| Field | Value |
| --- | --- |
| Reviewed revision (full hash) | `f8a769944e763f641a40ef707b9627e449f8f0fa` |
| Reviewed subject | `preserve structured values and lexical provenance` |
| Reviewer identity | independent kernel reviewer subagent |
| Review date (UTC) | 2026-08-03T18:01:32Z |
| Branch | `refact-runtime` |
| Scope | AUTH-01, AUTH-02, AUTH-05, APP-03, LANG-01..05, PROV-01/02, kernel QUALITY-01 / VERIFY-01 |
| Gate decision | **FAIL** |

### Preceding implementation revisions reviewed

| Order | Full hash | Subject |
| --- | --- | --- |
| 0 | `25805270b269cd1ecfd60d07ec6d7a7c091cf3bb` | `impl plan to solve problems tt1 on impl rfc runtime` |
| 1 | `bc75bad0e16dca0873b4b690f666651a959772e3` | `seal runtime history mutation and program verification` |
| 2 | `2ccb82da1f836b1af5f7b14f5c6fb5564cec9e21` | `make safe failure settlement total` |
| 3 | `f8a769944e763f641a40ef707b9627e449f8f0fa` | `preserve structured values and lexical provenance` |

HEAD after Commit 3 matches the required subject chain. No fixing revision exists yet for
open blockers found in this review.

### Commands and generated evidence reviewed

#### Implementer verification logs

| Log | Path | Disposition used |
| --- | --- | --- |
| Commit 1 focused verify + re-verify | `/tmp/grok-goal-649474994ccb/implementer/verify/commit-01.log` | package tests, causal suite, compile-fail, metadata |
| Commit 1 metadata re-check | `/tmp/grok-goal-649474994ccb/implementer/verify/commit-01-metadata.log` | `cargo-metadata-contract` ok |
| Commit 2 focused verify | `/tmp/grok-goal-649474994ccb/implementer/verify/commit-02.log` | program/certify/runtime/store safe-failure proofs |
| Commit 3 focused verify | `/tmp/grok-goal-649474994ccb/implementer/verify/commit-03.log` | program/spec/certify/runtime/store provenance proofs |

Notes from implementer logs:

- Commit 1 causal suite: 12/12 ok; `cargo-metadata-contract` ok; app/store compile-fail ok.
- Commit 2: success-only safe-failure compile-fails and process-handle totality tests ok.
- Commit 3: final package gates ok after an intermediate dirty-tree compile error in
  `failure_mapping_tests` (`structured_value_contract` import); fixed before commit tip.
  Independent re-check at HEAD confirms the failure-mapping tests compile and pass.

#### Independent reviewer re-checks (this review)

All commands run under the Nix development shell from repository root at
`f8a769944e763f641a40ef707b9627e449f8f0fa`.

| Command | Local evidence | Result |
| --- | --- | --- |
| `nix develop -c cargo test -p mfm-certify` | `/tmp/review-certify.log` | pass (lib 23, integration 38, ui 1, doctest 1) |
| `nix develop -c cargo test -p mfm-runtime` | `/tmp/review-runtime.log` | pass (caller-owned port smoke) |
| `nix develop -c cargo test -p mfm-store --features test-support` | `/tmp/review-store.log` | pass (unit 20, ui 1, runtime 4, causal 12) |
| `nix develop -c cargo test -p mfm-program` | `/tmp/review-program.log` | pass (authoring 6, ui 1, doctest 3) |
| `nix develop -c cargo test -p mfm-spec` | `/tmp/review-spec.log` | pass (normalization 5, contracts 2, doctest 1) |
| `nix run .#run -- --task cargo-metadata-contract` | `/tmp/review-metadata.log` | pass |

#### Repository-wide searches for deleted / sealed authority surfaces

Searches executed against the working tree excluding `target/`:

| Surface | Search | Result at HEAD |
| --- | --- | --- |
| Public production `StructuredRunHistoryWriter` | `StructuredRunHistoryWriter` | Type is `pub(super)` in `crates/kernel/store/src/structured/backend.rs`. External uses are compile-fail only (`writer_not_public`, app ui). |
| Writer-returning production `split` | `fn split` on run store | `StructuredRunStore::split` is `pub(super)`. Configuration `split` remains public for configuration streams (out of kernel AUTH-01 run-history scope). |
| `StructuredProgramVerifier` | name | Present only in compile-fail `program_verifier_trait_not_public`. Production seam is private `ProgramVerifier` trait in fold/adapter. |
| `StoreProgramVerifier` | name | Absent. |
| Public `VerifiedProgramData` construction | `VerifiedProgramData::new` | Constructor is `pub(super)`. Compile-fail `verified_program_data_not_constructible` passes. Type remains nameable. |
| `split_qualified_registry` | name | Absent from code (plan text only). Production consumes `QualifiedProgramRegistry::into_runtime_parts` inside store assembly. |
| `reviewed_safe_failures` sample-corpus API | name | Absent from Rust sources (problem text only). Totality uses typed success-only settlement. |
| Panic-path `from_declaration_ordered` | name | Replaced by `FanOutResults::try_from_declaration_ordered` returning `Option` (empty → `None`). |
| Caller-supplied admission `RunId` | `AdmitRunRequest`, `StructuredAdmissionCommand` | No run-id field on admission command/request. Store adapter derives via `mfm.run-id-preimage.v1`. |

Additional residual public surfaces (not alternate writers, but not fully private either):

- `mfm_store::structured` re-exports semantic proposal types and store-local
  `StructuredAppendAttempt` / `StructuredAdmissionRequest` for typing; admission request
  construction is `pub(super)`; production writer methods are unreachable outside the crate.
- `AdmissionVerificationRegistry` remains cloneable via
  `QualifiedProgramRegistry::admission_verification_registry(&self)`, but production adapter
  construction remains private and assembly consumes the complete registry once.

### Proof dispositions

#### AUTH-01 — Runtime is sole production run-history writer

**PASS**

Evidence:

- Dependency inversion: `mfm-runtime` and `mfm-certify` do not depend on `mfm-store`;
  `mfm-store` depends on both (`Cargo.toml` + `cargo-metadata-contract`).
- Production assembly (`assemble_structured_runtime`) returns `Runtime` + purpose readers only;
  writer stays inside private `StoreHistoryAdapter`.
- Compile-fail: `crates/kernel/store/tests/ui/fail/writer_not_public.rs`,
  app `run_history_writer_cannot_clone`, `run_history_reader_cannot_append`.
- Causal suite drives Pure/Read/Effect/fact/auth/observation/ambiguity/stale-head/root-close
  paths through Runtime (`structured_runtime_causal`, 12/12).
- Runtime unit test documents caller-owned port without production backend attachment.

#### AUTH-02 — Certified-program verification cannot be forged

**PASS**

Evidence:

- Concrete `AdmissionVerificationRegistry` installed only through store assembly /
  private adapter cache.
- `VerifiedProgramData::new` is `pub(super)`; public construction fails compile-fail.
- Hostile substitution: `exact_root_program_cache_rejects_authored_object_substitution`,
  `admission_requires_the_exact_certified_resource_lineage_set`.
- Certify UI: certified program not constructible/cloneable; process-handle registry exactness
  tests pass.
- `StructuredProgramVerifier` public trait deleted (compile-fail remains as tombstone).

#### AUTH-05 — Purpose-limited read capabilities

**FAIL (blocker)**

Required by plan/problem statement:

1. Distinct purpose authority for public-read, trace, audit, replay, export.
2. Each purpose exposes only its exact projection (not complete history).
3. A consumer holding one purpose cannot load/project another purpose’s data.
4. Compile-fail and policy tests cover cross-purpose substitution.
5. `AuthorizedRunCall` retains the approved grant; facade methods accept only the matching
   purpose-bound call.

Findings at HEAD:

| Required property | Observed at `f8a76994…` |
| --- | --- |
| Distinct purpose reader types | Present: `PublicRunReader`, `TraceRunReader`, `AuditRunReader`, `ReplayRunReader`, `ExportRunReader` in `crates/kernel/store/src/structured/purpose.rs`. |
| Purpose-limited projection | **Absent.** Every purpose reader’s `load` calls the same `load_verified` and returns full `VerifiedStructuredRun` with complete records/objects/auth/observation surfaces (`fold.rs` public accessors). |
| Cross-purpose impossibility | **Absent.** Holding any purpose reader yields complete verified history. |
| Production retains purpose-split readers | **Absent.** `production_structured.rs` keeps only `assembled.public_reader` and uses it for public, drive post-load, replay, transition-trace, access-audit, and export via `load_authorized`. |
| `AuthorizedRunCall` retains approved grant | **Absent.** Struct fields are credential/policy/scopes/run_id only — no retained `RunAccessGrant`. All backend methods take the same `AuthorizedRunCall`. |
| Compile-fail cross-purpose | Partial only: `public_reader_is_not_trace_reader` proves type inequality, not projection isolation. |
| Purpose reader cloneability | Inner `StructuredRunHistoryReader` remains `Clone`; purpose wrappers do not add projection sealing. |

This matches the still-open AUTH-05 problem shape in `PROBLEMS_TT1_IMPLRFC_RUNTIME.md`
(complete `load_verified`, one production reader, grant not retained on the call type).
Commit 1 claimed to close AUTH-05; the authority cutover for mutation/verification is real,
but purpose-limited **read** authority is not.

#### APP-03 — Store derives run ID; callers cannot admit with supplied digest

**PASS**

Evidence:

- `StructuredAdmissionCommand::new` documents and implements no caller run id.
- `AdmitRunRequest` wire/type has entry point, invocation identity, and input only.
- Store adapter `derive_run_id` encodes exactly the four annex fields through
  `mfm.run-id-preimage.v1`.
- Tests: `identical_run_id_preimages_derive_identical_ids`,
  `each_run_id_preimage_field_changes_identity` (`mfm-store` structured_runtime tests);
  integration `run_identity_contract` present.

#### LANG-01 — SafeFailure settlement total / success-only

**PASS**

Evidence:

- Distinct disposition types: `SafeFailureSuccessOnly` / `SafeFailureMayFail` with success-only
  proposal types that exclude Failure/InvalidEvidence.
- Compile-fail:
  - `success_only_safe_failure_cannot_fail`
  - `success_only_safe_failure_cannot_invalid_evidence`
  - `may_fail_safe_failure_cannot_invalid_evidence`
- Process-handle tests:
  - `fallible_success_only_safe_failure_always_settles_success`
  - `may_fail_safe_failure_settlement_is_total_over_arbitrary_valid_values`
  - `returned_value_settlement_retains_full_invalid_evidence_behavior`
  - `infallible_no_refresh_effect_settles_reviewed_safe_failure_as_success`
- Causal: `safe_failure_closes_through_default_mapping_without_blocking_an_unrelated_run`,
  adapter/callback fault tests preserve history without forged semantic failure authority.
- `reviewed_safe_failures` corpus path deleted from code.

#### LANG-02 — Typed child failure crosses declared failure exit

**PASS**

Evidence:

- `typed_child_failure_crosses_one_exact_fragment_boundary_into_the_parent_handler`
  (`mfm-certify` structured_certification).
- Fold rebinds child failure into certified boundary source rather than success/parent
  placeholder (`fold.rs` comments + child-boundary process-handle tests).

#### LANG-03 — Structured aggregates compose through constructs

**PASS**

Evidence:

- Sealed `StructuredValue` algebra + recursive `StructuredValueDefinition`.
- Non-empty `FanOutResults` head+tail with strict decode / `try_from_declaration_ordered`.
- Authoring: `match_merges_aggregate_results_and_state_returns_aggregate`,
  `fan_out_is_non_empty_and_depth_two_join_is_consumable`.
- Certification/pipeline aggregate fixtures and depth-two fan-out execution tests pass.

#### LANG-04 — Duplicate labels rejected

**PASS**

Evidence:

- Authoring rejects duplicate stable declaration labels, Match arm labels/tags, and fan-out
  lane keys (`program/src/structured.rs` error paths).
- Normalization rejects duplicate/out-of-order hostile tables
  (`normalized_program_is_strict_and_rejects_hostile_tables`).
- Failure-mapping rejects duplicate/cyclic plans.

#### LANG-05 — Qualified ordinals define canonical order

**PASS** (behavior present; explicit rename property test is thin)

Evidence:

- Declaration/lane ordinal assignment during authoring; path `Ord` is ordinal-first for
  declarations and fan-out lanes (`spec/src/structured.rs` comment + implementation).
- `declaration_order_and_semantic_ids_are_stable`.
- `certified_fan_out_join_is_declaration_ordered_under_completion_permutations`.
- Residual: no dedicated automated test that renames diagnostic labels while holding
  ordinals fixed and asserting path order identity. Not treated as a blocker because
  ordering code is ordinal-primary and declaration-order proofs exist.

#### PROV-01 — Match-arm / fan-out-lane provenance survives fold

**PASS** (kernel encode/fold; portable export/replay E2E residual under later VERIFY/REPLAY)

Evidence:

- Journal `LexicalValueRef.structural_origin` / `StructuralValueOrigin::{MatchArm,FanOutLane}`
  binds group path, ordinal, key, contract, and source refs independently of payload bytes.
- Fold sets origins on Match merge and FanOut lane completion (`fold.rs`).
- Store unit tests assert two identical lane payloads share content identity but retain
  distinct `lane_ordinal` origins; omission forgery rejected.
- Causal: `fan_out_structural_values_survive_fresh_persisted_folds`.
- Residual: plan text also names export/replay end-to-end survival; full portable replay
  ownership remains later commits (15–17). Kernel retained provenance on persisted fold
  state is demonstrated.

#### PROV-02 — Bounded iterative normalization rejects hostile depth

**PASS**

Evidence:

- Explicit work-stack normalization/denormalization with
  `MAX_STRUCTURAL_PATH_DEPTH` / `MAX_PROVENANCE_RESOLUTION_DEPTH`.
- Tests:
  - `provenance_resolution_accepts_maximum_depth_and_rejects_over_depth_without_stack_overflow`
  - `denormalize_value_rejects_hostile_json_node_budget_without_stack_overflow`
- Fan-out third-level compile-fail / authoring bounds remain enforced.

#### QUALITY-01 — Module responsibility splits (kernel portion)

**PASS with non-blocker residual**

Progress at HEAD:

- Runtime history split into `history/{port,commands,cursor,proofs,error,identity}`.
- Store structured surface split into adapter/assembly/backend/mutation/purpose/fold/… .
- Spec gained `structured_normalization_tests.rs` module file.

Residual (explicitly allowed to continue in later commits; **not a gate blocker**):

| Module | Approx. LOC | Status |
| --- | --- | --- |
| `crates/kernel/certify/src/structured.rs` | ~11.5k | still monolithic |
| `crates/kernel/store/src/structured/fold.rs` | ~4.6k | still oversized |
| `crates/kernel/spec/src/structured.rs` | ~3.8k | partial only |
| `crates/kernel/program/src/structured.rs` | ~3.0k | plan-requested value/state/block/Match/fan-out split not done |

Behavior proofs for Commits 1–3 exist despite incomplete physical module splits.

#### VERIFY-01 — Kernel boundary / hostile / compile-fail proofs (portion through Commit 3)

**FAIL (inherited from AUTH-05)**

Present and green:

- Writer/verifier/certified-program/safe-failure compile-fails.
- Causal Runtime mutation suite.
- Run-id preimage field matrix.
- Safe-failure totality and aggregate/provenance hostile-depth proofs.
- Cargo metadata acyclic ownership edges.

Missing / incomplete for claimed Commit 1 AUTH-05 closure:

- Projection-isolating purpose readers.
- Grant-retaining purpose-specific authorized call types wired to matching readers.
- Compile-fail/policy matrix proving one purpose cannot project another’s data
  (beyond type-name inequality).

Additional non-blocking VERIFY residuals:

- Not every Commit 1 bullet has a dedicated compile-fail (e.g. public construction of every
  proposal type / append attempt); authority is still sealed by private writer + private
  adapter construction.
- PROV export/replay portable E2E deferred with REPLAY commits.

### Findings

#### Blockers (gate remains FAIL while open)

| ID | Severity | Finding | Owning fix scope |
| --- | --- | --- | --- |
| B-AUTH-05-1 | **Blocker** | Purpose readers are type tags over complete `load_verified`; they do not expose purpose-limited projections. | Smallest AUTH-05 fix on store purpose readers + projections |
| B-AUTH-05-2 | **Blocker** | Production assembly discards non-public purpose readers and serves all read purposes through one `PublicRunReader`. | App production assembly + backend wiring |
| B-AUTH-05-3 | **Blocker** | `AuthorizedRunCall` does not retain the approved `RunAccessGrant`; facade methods share one call type unbound to a purpose reader. | App access/call types per plan Commit 1 |
| B-AUTH-05-4 | **Blocker** | Compile-fail coverage proves only `PublicRunReader ≠ TraceRunReader`, not “cannot project another purpose’s data”. | Purpose-specific projection APIs + compile-fail/policy tests |

#### Non-blocking residuals

| ID | Severity | Finding | Disposition |
| --- | --- | --- | --- |
| R-QUALITY-01 | Residual | Oversized `certify`/`fold`/`program`/`spec` structured modules remain. | Continue QUALITY-01 in later commits |
| R-PROV-01-E2E | Residual | Portable export/replay provenance survival not fully re-proved at this checkpoint. | Continue under VERIFY-01 / REPLAY commits |
| R-LANG-05-TEST | Residual | Ordinal-first ordering is implemented; dedicated label-rename property test is thin. | Optional strengthen-test only |
| R-SURFACE | Residual | Proposal/attempt types remain publicly nameable; writer/admission construction sealed. | Acceptable if no alternate mutation path (none found) |
| R-REGISTRY-CLONE | Residual | `admission_verification_registry(&self)` can snapshot admission half without consuming process half; production assembly still consumes complete registry privately. | Prefer later hardening; not an alternate production writer |

### Fixing revisions and re-review

| Event | Revision | Outcome |
| --- | --- | --- |
| Initial kernel review | `f8a769944e763f641a40ef707b9627e449f8f0fa` | **FAIL** — AUTH-05 blockers B-AUTH-05-1..4 open |
| Fixing revision(s) | _none yet_ | Implementer must fix AUTH-05 in the smallest owning implementation commit(s), rerun focused verification, then request re-review |
| Re-review revision | _pending_ | Required before any PASS checkpoint commit |

### Explicit gate decision

```text
GATE: FAIL
```

Rationale: Commits 1–3 deliver sealed production mutation/verification (AUTH-01/02), derived
run identity (APP-03), total safe-failure settlement (LANG-01), aggregate composition
(LANG-02..05), and retained/bounded provenance (PROV-01/02) with focused tests green at HEAD.
They do **not** complete AUTH-05 purpose-limited read authority as specified by the problem
statement and Commit 1 construction/proof requirements. Per plan Commit 4 rules, a checkpoint
PASS must not be recorded while a blocker remains open.

### Required before re-requesting this checkpoint

1. Seal purpose readers so each public method/projection cannot observe another purpose’s data
   (not merely distinct newtypes over complete history).
2. Wire production assembly to retain and use the matching purpose readers (or equivalent
   sealed purpose handles), not a single complete public reader for all purposes.
3. Make authorized call types retain the approved grant and bind facade methods to the matching
   purpose capability.
4. Add compile-fail and/or policy tests that fail closed on cross-purpose load/projection.
5. Rerun at least: store/app compile-fail surfaces, `mfm-store` tests with `test-support`,
   app structured UI tests, and any new AUTH-05 proofs.
6. Request independent re-review of the exact post-fix revision.

---

## Kernel checkpoint re-review (after AUTH-05 fix)

### Identity

| Field | Value |
| --- | --- |
| Re-review revision (full hash) | `e636b4d326e98b1647d512e28bf93d18fa6bbcc1` |
| Re-review subject | `seal purpose limited run history readers` |
| Prior FAIL revision | `f8a769944e763f641a40ef707b9627e449f8f0fa` |
| Reviewer identity | independent kernel reviewer subagent (AUTH-05 re-review) |
| Review date (UTC) | 2026-08-03T18:18:47Z |
| Branch | `refact-runtime` |
| Scope | AUTH-01, AUTH-02, AUTH-05, APP-03, LANG-01..05, PROV-01/02, kernel QUALITY-01 / VERIFY-01 |
| Gate decision | **PASS** |

### Preceding implementation revisions reviewed

| Order | Full hash | Subject |
| --- | --- | --- |
| 0 | `25805270b269cd1ecfd60d07ec6d7a7c091cf3bb` | `impl plan to solve problems tt1 on impl rfc runtime` |
| 1 | `bc75bad0e16dca0873b4b690f666651a959772e3` | `seal runtime history mutation and program verification` |
| 2 | `2ccb82da1f836b1af5f7b14f5c6fb5564cec9e21` | `make safe failure settlement total` |
| 3 | `f8a769944e763f641a40ef707b9627e449f8f0fa` | `preserve structured values and lexical provenance` |
| 4 (AUTH-05 fix) | `e636b4d326e98b1647d512e28bf93d18fa6bbcc1` | `seal purpose limited run history readers` |

Ancestry check: commits 0–3 and the AUTH-05 seal commit are all ancestors of HEAD
`e636b4d326e98b1647d512e28bf93d18fa6bbcc1` (verified with `git merge-base --is-ancestor`).

### Original FAIL findings (closed by this re-review)

Recorded at `f8a769944e763f641a40ef707b9627e449f8f0fa` in the prior kernel checkpoint:

| ID | Severity | Finding at FAIL HEAD |
| --- | --- | --- |
| B-AUTH-05-1 | Blocker | Purpose readers were type tags over complete `load_verified` returning public `VerifiedStructuredRun`. |
| B-AUTH-05-2 | Blocker | Production kept only `assembled.public_reader` and served all read purposes through it. |
| B-AUTH-05-3 | Blocker | `AuthorizedRunCall` did not retain the approved `RunAccessGrant`; facade methods shared one unbound call type. |
| B-AUTH-05-4 | Blocker | Compile-fail proved only `PublicRunReader ≠ TraceRunReader`, not projection isolation. |

### Fixing revision

| Field | Value |
| --- | --- |
| Fixing revision (full hash) | `e636b4d326e98b1647d512e28bf93d18fa6bbcc1` |
| Subject | `seal purpose limited run history readers` |
| Primary surfaces | `crates/kernel/store/src/structured/purpose.rs`, `assembly.rs`, `crates/app/src/production_structured.rs`, `crates/app/src/application.rs`, `crates/kernel/replay/src/structured.rs`, app UI compile-fails, docs |

### Independent re-check commands (this re-review)

All commands run under the Nix development shell from repository root at
`e636b4d326e98b1647d512e28bf93d18fa6bbcc1`.

| Command | Local evidence | Result |
| --- | --- | --- |
| `nix develop -c cargo test -p mfm-store --features test-support --lib` | `/tmp/review-store-rereview.log` | pass (20 unit) |
| `nix develop -c cargo test -p mfm-store --features test-support --test structured-runtime --test structured-runtime-causal` | `/tmp/review-store-auth05.log` | pass (4 + 12 causal) |
| `nix develop -c cargo test -p mfm-store --features test-support --test api-surface` | `/tmp/review-store-api-surface.log` | pass (writer/verifier/construct compile-fails) |
| `nix develop -c cargo test -p mfm-app --test application-privacy-ui` | `/tmp/review-app-ui.log` | pass (10 UI compile-fails including AUTH-05) |
| `nix develop -c cargo test -p mfm-replay` | `/tmp/review-replay.log` | pass (package builds; 0 unit tests) |

### AUTH-05 adversarial re-verification

Required properties and observed state at re-review HEAD:

| Required property | Observed at `e636b4d3…` | Disposition |
| --- | --- | --- |
| Distinct purpose reader types | `PublicRunReader`, `TraceRunReader`, `AuditRunReader`, `ReplayRunReader`, `ExportRunReader` in `purpose.rs`; assembly returns all five. | met |
| Purpose-limited projection (not public complete history) | Each reader load returns a sealed evidence newtype (`PublicRunEvidence`, `TraceRunEvidence`, `AuditRunEvidence`, `RecordedRunEvidence`, `ExportRunEvidence`). No public field, no `Deref`, private `from_verified`. Accessors are purpose-specific subsets (e.g. public has heads/frontier/object lookup, not `records()`; export has records/objects_through/cursor; trace/audit expose record chronology without export/cursor surfaces). Shared fold still uses internal `VerifiedStructuredRun` only inside private reader (`pub(super) StructuredRunHistoryReader::load_verified`). | met |
| Cross-purpose impossibility | Distinct evidence types; purpose load methods are not interchangeable (`load_public` vs `load_for_export` vs …). Replay projections take purpose-specific evidence (`project_transition_trace(&TraceRunEvidence)`, `project_access_audit(&AuditRunEvidence)`, `project_operation_outcome(&PublicRunEvidence)`). | met |
| Production retains and uses all five purpose readers | `ProductionBackend` fields: `public_reader`, `trace_reader`, `audit_reader`, `replay_reader`, `export_reader`. Connect retains all five from `assemble_structured_runtime`. Load paths: public/drive → `load_public`; trace → `load_transition_trace`; audit → `load_access_audit`; replay verify → `load_for_recorded_verify`; export (and replay reproduce/compare internal validation) → `load_for_export`. | met |
| `AuthorizedRunCall` retains grant and is purpose-typed | `AuthorizedRunCall<'policy, G: RunGrantMarker>` stores `grant: RunAccessGrant` plus `PhantomData<G>`. Markers: Drive, ReadPublic, InspectTrace, InspectAudit, Replay, Export. Facade/`ApplicationBackend` methods accept only matching `AuthorizedRunCall<'_, …>`. | met |
| Compile-fail cross-purpose isolation | App UI: `public_reader_is_not_trace_reader`, `public_reader_cannot_load_export_evidence`, `public_reader_cannot_load_for_export_method`, `public_evidence_is_not_export_evidence` all ok. Store: `purpose_readers_are_distinct_types` runtime assertion. | met |

Adversarial residuals checked and **not** treated as AUTH-05 blockers:

- Internal fold still materializes complete `VerifiedStructuredRun` before sealing; design contract documents this and forbids returning it from purpose APIs (`docs/run-execution.md`, store README).
- `VerifiedStructuredRun` remains publicly **nameable** with full accessors for Runtime port/`doc(hidden)` adapter use; production purpose readers never return it, and construction remains sealed by private fields. Residual class matches prior R-SURFACE (nameable without production complete-history authority).
- Compile-fail matrix samples public↔trace and public↔export rather than every ordered purpose pair; the isolation mechanism is uniform typed seals, so sample coverage is accepted.
- `replay_run(Reproduce/CompareCurrent)` internally loads `ExportRunEvidence` under a Replay-authorized call, then validates and projects a `ReplayResponse` only — callers never receive export evidence or the export facade without `RunAccessGrant::Export`.

### Proof dispositions (re-review)

| Proof | Disposition | Notes |
| --- | --- | --- |
| AUTH-01 | **PASS** (unchanged) | Writer/`split` still `pub(super)`; assembly returns Runtime + purpose readers only; causal suite 12/12. |
| AUTH-02 | **PASS** (unchanged) | Private admission verifier; `VerifiedProgramData::new` `pub(super)`; hostile cache/lineage tests green. |
| AUTH-05 | **PASS** (was FAIL) | B-AUTH-05-1..4 closed by `e636b4d3…`. Evidence above. |
| APP-03 | **PASS** (unchanged) | Store-derived run id; preimage field matrix tests green. |
| LANG-01 | **PASS** (unchanged) | Success-only settlement types + compile-fails + process-handle totality. |
| LANG-02 | **PASS** (unchanged) | Typed child failure boundary rebinding. |
| LANG-03 | **PASS** (unchanged) | Aggregate/fan-out composition. |
| LANG-04 | **PASS** (unchanged) | Duplicate label rejection. |
| LANG-05 | **PASS** (unchanged; thin rename property residual) | Ordinal-first path order. |
| PROV-01 | **PASS** (unchanged; portable E2E residual later) | Match/fan-out structural origin on fold. |
| PROV-02 | **PASS** (unchanged) | Bounded hostile-depth normalization. |
| QUALITY-01 | **PASS with residual** (unchanged) | Oversized certify/fold/program/spec modules remain for later commits. |
| VERIFY-01 (kernel through this checkpoint) | **PASS** (was FAIL inherited from AUTH-05) | AUTH-05 proofs now present and green; prior mutation/safe-failure/provenance proofs remain. |

### Blockers after re-review

None open for the kernel remediation checkpoint scope.

### Non-blocking residuals (carried forward)

| ID | Severity | Finding | Disposition |
| --- | --- | --- | --- |
| R-QUALITY-01 | Residual | Oversized `certify`/`fold`/`program`/`spec` structured modules remain. | Later QUALITY-01 commits |
| R-PROV-01-E2E | Residual | Portable export/replay provenance survival not fully re-proved here. | Later VERIFY/REPLAY commits |
| R-LANG-05-TEST | Residual | Ordinal-first ordering implemented; dedicated label-rename property test thin. | Optional strengthen-test |
| R-SURFACE | Residual | `VerifiedStructuredRun` and some proposal/attempt types remain nameable; production purpose readers do not return complete verified history; writer/admission construction sealed. | Acceptable residual |
| R-REGISTRY-CLONE | Residual | `admission_verification_registry(&self)` can snapshot admission half; production assembly still consumes complete registry privately. | Prefer later hardening |
| R-AUTH-05-CF-MATRIX | Residual | Cross-purpose compile-fails sample public vs trace/export rather than full 5×5 method matrix. | Type seals cover mechanism; optional expand |

### Fixing revisions and re-review ledger

| Event | Revision | Outcome |
| --- | --- | --- |
| Initial kernel review | `f8a769944e763f641a40ef707b9627e449f8f0fa` | **FAIL** — AUTH-05 blockers B-AUTH-05-1..4 open |
| Fixing revision | `e636b4d326e98b1647d512e28bf93d18fa6bbcc1` | Seals purpose readers/evidence, production five-reader wiring, grant-retaining purpose-typed `AuthorizedRunCall`, cross-purpose compile-fails |
| Re-review revision | `e636b4d326e98b1647d512e28bf93d18fa6bbcc1` | **PASS** — B-AUTH-05-1..4 closed; other kernel proofs reconfirmed |

### Explicit gate decision

```text
GATE: PASS
```

Rationale: HEAD `e636b4d326e98b1647d512e28bf93d18fa6bbcc1` includes commits 0–3 plus the AUTH-05 fix.
Independent re-checks of store (lib, structured-runtime, causal, api-surface), app privacy UI
compile-fails, and replay package build are green. Purpose-limited read authority now matches
the problem/plan requirements: sealed purpose evidence (not public complete history), all five
production purpose readers held and used, grant-retaining purpose-typed authorized calls, and
compile-fail cross-purpose isolation. Prior AUTH-01/02, APP-03, LANG-01..05, and PROV-01/02
PASS dispositions remain valid. Non-blocking QUALITY/E2E residuals do not reopen this gate.

---

_End of kernel checkpoint re-review. Gate PASS recorded at exact HEAD
`e636b4d326e98b1647d512e28bf93d18fa6bbcc1`._

---

## PostgreSQL store checkpoint (after Commit 7)

### Identity

| Field | Value |
| --- | --- |
| Reviewed revision (full hash) | `67e9a549c3aa908e4c957289bcb482ab57961d34` |
| Reviewed subject | `bound postgres fact publication and query assurance` |
| Reviewer identity | independent postgres remediation reviewer subagent |
| Review date (UTC) | 2026-08-03T19:15:00Z |
| Branch | `refact-runtime` |
| Scope | AUTH-03, AUTH-04, STORE-01..07, related QUALITY-01 / VERIFY-01 |
| Gate decision | **PASS** |

### Preceding implementation revisions reviewed

| Order | Full hash | Subject |
| --- | --- | --- |
| 5 | `1d015b05c29ff8c3963d0c3e14574f35ad5b303b` | `seal postgres store target authority` |
| 6 | `54de9296b45387004768de5ff2c38236f44e7e15` | `classify postgres exact head races precisely` |
| 7 | `67e9a549c3aa908e4c957289bcb482ab57961d34` | `bound postgres fact publication and query assurance` |

HEAD after Commit 7 matches the required subject chain. Kernel checkpoint at
`e636b4d326e98b1647d512e28bf93d18fa6bbcc1` remains recorded above and is not reopened.

### Commands and generated evidence reviewed

#### Implementer verification logs

| Log | Path | Disposition used |
| --- | --- | --- |
| Commit 5 recoverability | `/tmp/grok-goal-649474994ccb/implementer/verify/commit-05.log` | `recoverability-postgres-v1` ok |
| Commit 5 sqlx online | `/tmp/grok-goal-649474994ccb/implementer/verify/commit-05-sqlx.log` | `postgres-sqlx-check` ok |
| Commit 5 integration compile trail | `/tmp/grok-goal-649474994ccb/implementer/verify/commit-05-integration*.log` | intermediate dirty-tree noise; HEAD re-checked independently |
| Commit 6 recoverability | `/tmp/grok-goal-649474994ccb/implementer/verify/commit-06.log` | `recoverability-postgres-v1` ok |
| Commit 7 recoverability + offline/inventory | `/tmp/grok-goal-649474994ccb/implementer/verify/commit-07.log` | recoverability + offline ok |
| Commit 7 inventory | `/tmp/grok-goal-649474994ccb/implementer/verify/commit-07-inventory.log` | `postgres-sql-inventory-check` ok |
| Commit 7 offline | `/tmp/grok-goal-649474994ccb/implementer/verify/commit-07-offline.log` | `postgres-sqlx-offline-check` ok |
| Commit 7 sqlx online | `/tmp/grok-goal-649474994ccb/implementer/verify/commit-07-sqlx.log` | `postgres-sqlx-check` ok |

#### Independent reviewer re-checks (this review)

All commands run under the Nix development shell / Nixfied tasks from repository root at
`67e9a549c3aa908e4c957289bcb482ab57961d34`.

| Command | Local evidence | Result |
| --- | --- | --- |
| `nix run .#run -- --task postgres-sql-inventory-check` | `/tmp/review-pg-inventory.log` | pass |
| `nix run .#run -- --task recoverability-postgres-v1` | `/tmp/review-pg-recoverability.log` | pass |
| `nix run .#run -- --task postgres-sqlx-check` | `/tmp/review-pg-sqlx.log` | pass |
| `nix develop -c cargo test -p mfm-store --features test-support --lib` | `/tmp/review-store-lib.log` | pass (21 tests, includes shared canonical-append bounds) |
| `nix develop -c cargo test -p mfm-storage-postgres --lib sql_inventory` | shell | pass (`every_runtime_query_is_owned_by_the_allowlist`) |

### Repository-wide searches for deleted / sealed authority surfaces

Searches executed against the working tree excluding `target/`:

| Surface | Search | Result at HEAD |
| --- | --- | --- |
| Production openers accepting `PgPool` / URL / fence | `open_structured_*`, `PgPool` in `qualification.rs` | Openers accept only opaque `ApplicationTargetSessions` / `CombinedTargetSessions` / `ConfigurationMaintenanceSessions`. No pool/URL/fence parameters. |
| Public pool getter / retained backend pool | `fn pool(`, `pub.*PgPool` in production sources | `RoleSession::pool` is `pub(crate)` only. Session bundle fields are private. Backend constructors are `pub(crate)`. |
| Global cluster roles (`mfm_store_application`, …) | `mfm_store_application`, `mfm_store_` | Absent. Roles are exact-target `mfm_t_{16hex}_{own\|qlf\|rrd\|rwr\|crd\|cwr}` from schema key. |
| Always-successful test fence | `TestAuthoritativeWriterFence`, `AlwaysSuccessful` | Absent from production Rust sources. |
| Public DML / locked write transaction | `LockedWriteTx`, `WriteTx` | `pub(crate)` in private `transaction` module only. |
| Shared writer pool backing readers | session construction | Distinct login/session pools per `SessionKind` (run-reader, run-writer, configuration-reader, configuration-writer). |
| Application connection construction | `crates/app` production path | `production_structured::connect` takes `ApplicationTargetSessions` only and calls `open_structured_authoritative_application`. |

Deployment-facing residual surfaces (explicit TCB, not ordinary application authority):

- Public `SessionLoginMaterial { database_url }` and `issue_*_sessions` are the deployment issuance boundary. Holders of those credentials are documented as inside the TCB (`crates/storages/postgres/README.md`, schema header comments).
- `PostgresSchema::migrate(database_url)` remains the owner migration path.

### Role / ACL matrix evidence

| Capability | Role set | Grant surface (schema-local) | Evidence |
| --- | --- | --- | --- |
| Owner / DDL | `mfm_t_*_own` | schema + table owner | `migrations/0001_store.sql` role create + `ALTER … OWNER` |
| Qualification | `mfm_t_*_qlf` | SELECT on catalog/history/config tables | ACL block + issuance `SET LOCAL ROLE` to qlf for target load |
| Run reader | `mfm_t_*_rrd` | SELECT run heads/batches/objects + fact heads/publications | ACL; `SessionKind::RunReader` → read-only tx |
| Run writer | `mfm_t_*_rwr` | SELECT/INSERT(/UPDATE heads) on run + fact tables only | ACL; no configuration DML grants |
| Configuration reader | `mfm_t_*_crd` | SELECT configuration revisions/heads | ACL; read-only tx |
| Configuration writer | `mfm_t_*_cwr` | SELECT/INSERT revisions + UPDATE heads | ACL; maintenance issuance only |

Sibling-target isolation (structural):

- Target key = `substr(md5(schema_name), 1, 16)`; sibling schemas get distinct role names and grants only on their schema (`0001_store.sql` comments + ACL `GRANT USAGE ON SCHEMA %I` for that schema only).
- Issuance probes exact membership set `{qualification, managed}` with `SET` allowed only for those two, rejects superuser/`BYPASSRLS`/inherit/admin-option/extra membership (`session.rs` `probe_login_shape`; tests `qualification_rejects_extra_membership_inheritance_and_admin_session_substitution`, `qualification_rejects_public_and_hostile_schema_or_table_grants`).

Per-transaction permit:

- Every read/write begins in `transaction.rs`, selects managed role, pins `search_path`, and compares database OID, schema, store scope/epoch, schema contract version, fence generation, and release epoch against the binding (`validate_target_permit`). Writers take an advisory fence lock before observing authority.

Role-matrix residual: no single exhaustive table-driven test enumerates every role × every operation × every sibling schema cell. Isolation is enforced by grant construction + issuance probes + purpose-separated session kinds; matrix completeness remains a residual (R-AUTH-04-MATRIX).

### Cross-process traces

| Trace | Observation | Disposition |
| --- | --- | --- |
| Fresh-process admit → continue | `structured_history_fresh_process_worker` + `fresh_process_refolds_and_continues_the_same_structured_run` spawns a second process with only opaque session env materials; second process refolds and continues without memory fallback | **present** |
| Fresh-process vs memory parity | Same fixture continues through memory and reopens PostgreSQL; closed frontiers match | **present** |
| Missing deployment credentials | Invalid URLs fail issuance with `PostgresStoreError::Connection`; no memory fallback | **present** |
| Same-run multi-process exact-head barriers (deterministic join of two OS processes racing one append identity) | Not present as a dedicated barrier harness; configuration and fact races are primarily same-process `tokio::join!` against one store | **partial residual** (R-STORE-03-XPROC) |
| Sequential reopen after durable append | Multiple tests reopen via `application_sessions()` / `issue_application_sessions` and continue | **present** |

### Fault-stage matrix

Plan Commit 7 stages: before lock, after lock, after decision read, after each DML class, before commit, after server commit / before acknowledgement, retry.

| Stage | Evidence at HEAD | Disposition |
| --- | --- | --- |
| Child object-row INSERT failure mid-append | `object_row_failure_rolls_back_batch_objects_and_head` (trigger inject); zero retained batch/object/head rows | **present** |
| Configuration head CAS / constraint failure | `configured_value_head_update_is_atomic_and_target_isolated` injects CHECK; revision count stays 1, head stays 1 | **present** |
| Coordinated configuration rollback visibility | `coordinated_configuration_rollback_is_visible_to_fresh_sessions` | **present** |
| Malformed retained objects post-qualification | `malformed_object_rows_fail_closed_after_qualification` | **present** |
| Connection loss / invalid credentials | Missing-deployment issuance fail-closed | **present (issuance)** |
| Full stage-by-stage inject for run append (before lock … post-commit ack loss + retry classification) | No complete harness covering every listed stage for run history | **partial residual** (R-STORE-03-FAULT) |
| Acknowledgement-unknown classification path | `CommitOutcome::AcknowledgementUnknown` in `transaction.rs`; mapped from both run and configuration append | **code present**; dedicated ack-loss retry suite thin |

Atomicity property for exercised stages: injected DML failure leaves no partial append (rollback under one locked write transaction). Residual is matrix breadth, not production dual-write ownership.

### SQL inventory

| Item | Evidence |
| --- | --- |
| Inventory module | `crates/storages/postgres/src/sql_inventory.rs` |
| Allowlist owners | Role/tx setup (`SET LOCAL ROLE`, isolation, `search_path`, advisory locks), permit/catalog reads, run/config/fact DML families, migration/role probes, `AssertSqlSafe` |
| Executable leaf | `postgres-sql-inventory-check` → `sql_inventory::tests::every_runtime_query_is_owned_by_the_allowlist` |
| Dynamic `sqlx::query(` count (src) | configuration 6, session 12, transaction 10, schema 8, structured 15, inventory self 2 |
| Checked `sqlx::query!` macros | **none** at HEAD (known residual R-STORE-05-MACROS) |
| Documentation honesty | README + `docs/build-and-verification.md` state inventory + offline/online prepare check; do not claim compile-time coverage of dynamic families |

Online `postgres-sqlx-check` still owns migration metadata prepare-check, authoritative schema probe, and hostile schema-mutation rejection (`SchemaAuthorityMismatch` path).

### Query-plan / fact-frontier evidence

| Property | Code / schema | Test evidence |
| --- | --- | --- |
| Atomic fact frontier on head row | `tenant_fact_heads` stores `fact_order`, `publication_count`, `minimum_order`, `maximum_order`; publication CAS updates head with publication insert in one locked write | `tenant_fact_publications_are_dense_atomic_and_exactly_routed`; concurrent publication/barrier linearization |
| No lifetime aggregate on publish | Publish path reads locked head + `EXISTS` ahead probe on indexed publications; no `COUNT`/`MIN`/`MAX` over lifetime history in `structured.rs` append | Code inspection |
| Bounded scan | `scan_fact_publications` filters PK prefix + `fact_order` range + fixed `LIMIT` (`maximum_items` ≤ 1024) | Prior-run fact scan reopen/empty-frontier tests |
| Index ownership | PK `(store_scope_id, store_epoch, tenant_scope_id, fact_order)` on `tenant_fact_publications` | Migration |
| Long-history `EXPLAIN` stability proof | Not present | **residual** R-STORE-06-PLAN |

### Proof dispositions

| Proof | Disposition | Notes |
| --- | --- | --- |
| AUTH-03 | **PASS** | Ordinary production cannot obtain/retain a cloneable pool through openers or session bundles. Deployment login materials remain TCB. Invalid credentials fail closed without memory fallback. |
| AUTH-04 | **PASS** (matrix residual) | Exact-target roles and schema-local grants; sibling isolation structural; hostile membership/inheritance/public-grant rejection tests green. Full role×op×sibling table residual. |
| STORE-01 | **PASS** (xproc residual) | Lock-before-decision + `classify_existing_under_lock`; healthy contention maps to `ExistingSame` / `AppendConflict` / `StaleHead`. Configuration concurrent matrix exact; multi-process run barriers partial. |
| STORE-02 | **PASS** | Always-successful production fence removed; no production-compilable always-approve fence type. |
| STORE-03 | **PASS** (matrix residual) | Shared Runtime-observable outcomes against PostgreSQL + memory parity/fresh process; fault/xproc matrices partial but atomic ownership is single private transaction module. |
| STORE-04 | **PASS** | Shared `canonical_append` bounds/closure validation before either backend DML; memory + postgres consume it; bounds unit test matches 16 MiB / 65_536. |
| STORE-05 | **PASS** (macro residual) | Executable dynamic allowlist + truthful docs; fixed SQL not yet fully `query!` migrated. |
| STORE-06 | **PASS** (plan residual) | Head-maintained frontier/count; bounded indexed scans; no lifetime aggregates on hot path. No long-history EXPLAIN harness. |
| STORE-07 | **PASS** | One private `transaction` module owns begin/role/`search_path`/permit/locks/commit classification for run and configuration. |
| QUALITY-01 (postgres portion) | **PASS with residual** | Split into `roles` / `session` / `transaction` / `sql_inventory`; `structured.rs` remains large. |
| VERIFY-01 (postgres portion) | **PASS with residual** | Inventory, sqlx, recoverability leaves green; incomplete multi-process barrier and full fault-stage harnesses remain. |

### Adversarial checks against FAIL criteria

| FAIL criterion | Observed | Gate impact |
| --- | --- | --- |
| Production can still bypass with retained pool | No public pool openers; session pools not extractable outside crate; app connect takes opaque sessions only | does not fail |
| Production can still use global roles | Global application/maintenance roles deleted; per-target role names only | does not fail |
| Healthy exact-head contention returns only availability | Contention SQLSTATEs re-read under lock; configuration race asserts exact domain outcomes without `BackendUnavailable` | does not fail |
| Dual transaction authority | Single `transaction.rs` owner | does not fail |
| Documentation claims broader SQL assurance than executable | Inventory + prepare-check boundary documented accurately | does not fail |

Known implementer residuals accepted as non-blocking when structure enforces the property and some proofs exist:

1. multi-process exact-head barriers partial
2. fault-injection stage matrix partial
3. not all SQL is `query!` macros

### Findings and fixing revisions

No open blockers found at this checkpoint. Non-blocking residuals:

| ID | Severity | Finding | Disposition |
| --- | --- | --- | --- |
| R-AUTH-04-MATRIX | Residual | Role/ACL matrix and sibling-target denial are structural + partial hostile tests; not a full combinatorial suite | Strengthen later under VERIFY-01 |
| R-STORE-03-XPROC | Residual | Cross-process same-run exact-head barriers incomplete; sequential fresh-process and same-process races present | Continue STORE-03 proofs |
| R-STORE-03-FAULT | Residual | Stage-by-stage fault inject covers object-row and configuration-head paths, not every planned run-append stage / ack-loss retry | Continue STORE-03 proofs |
| R-STORE-05-MACROS | Residual | Dynamic `sqlx::query` remains; owned by allowlist rather than `query!` | Optional migrate fixed shapes |
| R-STORE-06-PLAN | Residual | No long-history EXPLAIN/query-plan assertion | Optional performance harness |
| R-QUALITY-01-PG | Residual | `structured.rs` / large integration test file remain concentrated | Later QUALITY-01 |
| R-README-CLAIM | Residual | README claims “sibling-target denial” suite more broadly than dedicated multi-schema tests currently prove | Prefer wording aligned with structural+partial proofs or add the tests |
| R-DEPLOY-TCB | Residual | Public `SessionLoginMaterial` URLs can still open raw SQL if held; declared deployment TCB | Accept by contract; keep materials out of ordinary app paths |

### Fixing revisions and re-review ledger

| Event | Revision | Outcome |
| --- | --- | --- |
| Commit 5 authority seal | `1d015b05c29ff8c3963d0c3e14574f35ad5b303b` | Target roles, opaque sessions, private transaction module, fence removal |
| Commit 6 exact-head classification | `54de9296b45387004768de5ff2c38236f44e7e15` | Shared canonical append + lock-order classification |
| Commit 7 facts + SQL assurance | `67e9a549c3aa908e4c957289bcb482ab57961d34` | Atomic fact heads, inventory leaf, bounded scans |
| Independent review | `67e9a549c3aa908e4c957289bcb482ab57961d34` | **PASS** — no production retained-pool / global-role bypass; residuals non-blocking |

### Explicit gate decision

```text
GATE: PASS
```

Rationale: HEAD `67e9a549c3aa908e4c957289bcb482ab57961d34` closes AUTH-03/04 and STORE-01..07 for production authority, atomicity, and contention properties. Ordinary code cannot open history through a retained pool or global role; write authority is exact-target, permit-checked, and funneled through one private transaction typestate; healthy races classify domain outcomes; both backends share canonical ingress bounds; fact frontiers are head-maintained and scans are page-bounded; SQL assurance is executable and truthfully scoped. Partial multi-process barrier, fault-stage, EXPLAIN, and `query!` migration matrices remain residuals and do not reopen a production bypass.

Independent re-checks of `postgres-sql-inventory-check`, `recoverability-postgres-v1`, `postgres-sqlx-check`, and store lib tests are green at the reviewed revision.

---

_End of PostgreSQL store checkpoint. Gate PASS recorded at exact HEAD
`67e9a549c3aa908e4c957289bcb482ab57961d34`._
