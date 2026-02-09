# REDESIGN_PLAN.md Critique

## Blockers (Internal Inconsistencies / High Likelihood of Failure)

1. “No secrets in artifacts” contradicts “full context snapshots” unless you define/implement secret handling first.
   - Plan: “no secrets in events/manifests/artifacts by default” (`REDESIGN_PLAN.md:17`), and full context snapshot artifacts (`REDESIGN_PLAN.md:275`, `REDESIGN_PLAN.md:284`).
   - Redesign: “avoid secrets in artifacts; if unavoidable, artifacts must be encrypted” (`REDESIGN.md:655`, `REDESIGN.md:656`).
   - Missing: an explicit policy that makes this coherent (forbid secrets structurally, redact, or encrypt snapshots/artifacts).

2. State-attempt “single append” plus fact recording is crash-unsafe as written.
   - Plan: “entered + domain + completed/failed in a single append” (`REDESIGN_PLAN.md:256`), and record facts during live IO (`REDESIGN_PLAN.md:305` to `REDESIGN_PLAN.md:309`).
   - Failure mode: crash after IO returns but before the single append; fact is not durably discoverable, replay/resume breaks, and IO can be repeated.
   - Redesign requirements include idempotency/side-effect patterns (`REDESIGN.md:1006`, `REDESIGN.md:1014`, `REDESIGN.md:736`), but the plan doesn’t address the “IO happened but wasn’t recorded” window.

3. “Crash/resume determinism” tests aren’t achievable with the proposed fast-lane stores.
   - Plan: PR 07 proposes `InMemoryEventStore` (`REDESIGN_PLAN.md:186`, `REDESIGN_PLAN.md:187`).
   - Plan: PR 15 requires “simulate crash after N events, resume” in fast lane (`REDESIGN_PLAN.md:364` to `REDESIGN_PLAN.md:370`).
   - Issue: in-memory storage can’t simulate a persistence boundary; restart loses the event stream.
   - Needed: a persistent fast-lane event store (sqlite or file log) or redefine “crash simulation” to accept it won’t exercise real persistence semantics.

## Major Gaps vs `REDESIGN.md` (Risk of Implementing the Wrong Thing)

1. Build provenance is part of the public contract but missing from the implementation plan.
   - `RunManifest` includes `BuildProvenance` (`REDESIGN.md:969` to `REDESIGN.md:978`) and is in the stable public API contract (`REDESIGN.md:958`).
   - Plan mentions “store manifest artifact” (`REDESIGN_PLAN.md:263`) but never schedules collecting/storing provenance.

2. Retry/attempt semantics exist in the kernel event contract but are not planned.
   - Contract: `KernelEvent::StateEntered { attempt: u32, ... }` (`REDESIGN.md:1177`).
   - Contract: `RunConfig` includes retry policy, skip tags, event profile, checkpointing, and execution mode (`REDESIGN.md:919` to `REDESIGN.md:956`).
   - Plan: PR 10 doesn’t mention attempts, retry policy, skip-tags behavior, event profile gating, or checkpointing semantics (`REDESIGN_PLAN.md:248` to `REDESIGN_PLAN.md:274`).

3. Idempotency/dedupe is required by the redesign but not addressed in the plan’s tests or scope.
   - Redesign tests: “idempotency/dedupe tests for side-effect patterns” (`REDESIGN.md:736`).
   - Plan success criteria: replay/crash/concurrency only (`REDESIGN_PLAN.md:21` to `REDESIGN_PLAN.md:25`).
   - Plan minimal op is read-only and doesn’t force the hard parts (`REDESIGN_PLAN.md:362`, `REDESIGN_PLAN.md:363`).

## Document-Level Critique

1. Success criteria aren’t measurable enough to know “done” beyond compilation.
   - “Implement core invariants” is a slogan list unless tied to explicit API and test assertions (`REDESIGN_PLAN.md:10` to `REDESIGN_PLAN.md:17`).
   - The three design-level tests listed are necessary but insufficient to validate several invariants (secrets, canonicalization edge cases, artifact integrity, idempotency).

2. “Decision Defaults (Locked In)” locks in implementation choices before core semantics are proven.
   - Postgres + MinIO can be the end targets (`REDESIGN_PLAN.md:42` to `REDESIGN_PLAN.md:45`), but “locked in” this early increases wasted integration effort while semantics evolve.
   - The redesign migration plan puts Postgres+MinIO later, after semantics stabilize (`REDESIGN.md:752`).

3. You lock some choices while leaving equally critical semantics ambiguous.
   - Example: replay behavior for calls without `fact_key` is “error or non-replayable depending on policy” (`REDESIGN_PLAN.md:321` to `REDESIGN_PLAN.md:323`).
   - That’s a core determinism contract and should have a default rule.

4. “All CI parity commands pass” as acceptance is necessary but not sufficient.
   - Many PRs need behavioral acceptance tests (semantic invariants), not only compilation/lints (`REDESIGN_PLAN.md:85` to `REDESIGN_PLAN.md:88`, `REDESIGN_PLAN.md:140` to `REDESIGN_PLAN.md:144`, `REDESIGN_PLAN.md:158` to `REDESIGN_PLAN.md:160`).

## PR-by-PR Critique

1. PR 01 (Move only) is high churn and will create noisy diffs everywhere.
   - Survivable, but it makes subsequent review/backports harder.
   - Steps/acceptance should include doc/tooling path updates, not just Cargo+flake (`REDESIGN_PLAN.md:69` to `REDESIGN_PLAN.md:89`).

2. PR 01 directory naming is inconsistent and will fossilize.
   - `crates/machine_legacy` vs `crates/machine-derive_legacy` mixes underscore/hyphen (`REDESIGN_PLAN.md:78`, `REDESIGN_PLAN.md:79`).
   - Pick one convention now to avoid repeated renames.

3. PR 01 acceptance uses a placeholder that will rot.
   - “`nix run .#... -- --help`” should name the actual flake app attribute (`REDESIGN_PLAN.md:88`).

4. PR 02 (Hyphen package names) is directionally correct, but CLI naming is under-specified.
   - Today, package is `mfm` and bin is `mfm_cli` (see `mfm_cli/Cargo.toml`).
   - Plan: “rename legacy CLI package if desired” (`REDESIGN_PLAN.md:100`) is not really optional; it affects install, docs, and flake targets.
   - Your own checklist admits this is unresolved (`REDESIGN_PLAN.md:442`).

5. PR 03 (Remove `core -> machine`) is correct but likely underestimated and may be API-breaking.
   - Today `mfm_core` depends on `mfm_machine` (`mfm_core/Cargo.toml`).
   - If `mfm_core` exports machine types publicly, PR 03 breaks users; plan should state if breaking is acceptable or how compatibility is maintained (`REDESIGN_PLAN.md:108` to `REDESIGN_PLAN.md:122`).

6. PR 04 (New `mfm-machine` contract) is the right idea but too broad for one PR.
   - “Implement modules from Appendix C” is a lot (`REDESIGN_PLAN.md:135` to `REDESIGN_PLAN.md:138`).
   - For PR-sized work, split by module groups to keep review manageable.

7. PR 05 (Canonical JSON + hashing) is under-tested and may be in the wrong visibility boundary.
   - Tests only mention key-order invariance (`REDESIGN_PLAN.md:155`).
   - Redesign constraints: no NaN/Infinity; avoid floats in hashed structures (`REDESIGN.md:70`, `REDESIGN.md:71`).
   - If other crates need canonical hashing helpers, “internal only” will cause duplication or leaky APIs.

8. PR 06 (New derive macros) is premature.
   - Proc-macros are high maintenance and likely to churn while the contract stabilizes (`REDESIGN_PLAN.md:162` to `REDESIGN_PLAN.md:175`).
   - Better after at least one op crate is proven with the new machine types.

9. PR 07 (Fast lane stores) is missing the store needed for crash/resume tests.
   - Redesign suggests sqlite or similar for fast lane (`REDESIGN.md:642`).
   - Plan uses in-memory event store (`REDESIGN_PLAN.md:187`) which can’t validate crash/resume semantics.

10. PR 07 tests don’t cover integrity on read (content-addressing enforcement).
   - You test de-dupe by hash (`REDESIGN_PLAN.md:192`) but not “hash on read” and corruption errors (`REDESIGN.md:1102` to `REDESIGN.md:1104`).

11. PR 08 (Postgres schema locked in) is too minimal for real ergonomics and migration story is deferred.
   - Only `run_events` table is specified (`REDESIGN_PLAN.md:204` to `REDESIGN_PLAN.md:210`).
   - No runs table, no useful indexes, and migrations approach is “confirm later” (`REDESIGN_PLAN.md:444`).
   - Minimal is fine if accepted, but then run listing and resume can get awkward/slow unless indexed elsewhere.

12. PR 08 append semantics are vulnerable to race conditions unless you specify the SQL approach.
   - “check head then insert” must lock/serialize correctly or map conflicts deterministically to `StorageError::Concurrency` (`REDESIGN_PLAN.md:213` to `REDESIGN_PLAN.md:218`).
   - Plan doesn’t specify how.

13. PR 09 (S3 artifact store) ignores verification and metadata.
   - Content-addressed storage should validate bytes against the requested `ArtifactId` and return corruption errors; plan doesn’t mention it (`REDESIGN_PLAN.md:229` to `REDESIGN_PLAN.md:243`).

14. PR 10 (Engine skeleton) omits most of the semantic “dragons.”
   - Missing: attempt/retry loops, skip-tags behavior, event profile gating, checkpointing policy, and idempotency enforcement (`REDESIGN_PLAN.md:248` to `REDESIGN_PLAN.md:274`).
   - Also missing: explicit handling of the “IO happened but not recorded” crash window.

15. PR 10 “single append per attempt” is not justified and may be too strict.
   - Appendix C only requires domain events emitted within a state to be committed atomically with the state transition (`REDESIGN.md:1306`).
   - The plan’s stronger constraint can make IO recording crash-unsafe (`REDESIGN_PLAN.md:256`).

16. PR 11 (Snapshotting) doesn’t specify how determinism is ensured for snapshots.
   - Contract says deterministic serialization is required (`REDESIGN.md:1126` to `REDESIGN.md:1129`).
   - Plan doesn’t say how to enforce stable key ordering and number policies (`REDESIGN_PLAN.md:275` to `REDESIGN_PLAN.md:291`).

17. PR 12 (LiveIo) duplicates sources of truth between IO results and events.
   - Appendix C already models `IoResult.recorded_payload_id` (`REDESIGN.md:1274` to `REDESIGN.md:1280`).
   - Plan also wants `FactRecorded` domain events (`REDESIGN_PLAN.md:305` to `REDESIGN_PLAN.md:309`).
   - Pick a canonical source of truth and justify the other (auditability vs lookup speed vs correctness).

18. PR 13 (ReplayIo) leaves core semantics undecided and likely O(n).
   - The replay policy for missing `fact_key` is ambiguous (`REDESIGN_PLAN.md:321` to `REDESIGN_PLAN.md:323`).
   - “lookup recorded fact from run’s domain events” is likely O(n) per call unless you build an index once (`REDESIGN_PLAN.md:324`).

19. PR 14 (SDK planning) ignores op schema versioning constraints.
   - Redesign: every op must have explicit version; manifests/events include `op_id` + `op_version` (`REDESIGN.md:772` to `REDESIGN.md:776`).
   - Plan focuses on IDs only (`REDESIGN_PLAN.md:344` to `REDESIGN_PLAN.md:351`).

20. PR 15 (Minimal op) is too minimal to validate the hard parts.
   - It tests fact recording and output hashing, but doesn’t test ApplySideEffect + idempotency/dedupe required by redesign tests (`REDESIGN.md:736`).

21. PR 16 (New CLI) is underspecified on schema stability tests.
   - Redesign requires JSON schema stability tests for CLI/API responses (`REDESIGN.md:728`, `REDESIGN.md:729`).
   - Plan only says “CLI e2e tests” without committing to schema tests (`REDESIGN_PLAN.md:394`).

22. PR 16 mixes “thin orchestrator” with “legacy keystore commands remain” without an explicit boundary.
   - Plan says legacy keystore remains temporarily or is migrated “without touching event store” (`REDESIGN_PLAN.md:395`, `REDESIGN_PLAN.md:396`).
   - That needs a concrete approach (separate subcommand tree; explicit tests asserting no store calls).

23. PR 18 (REST API) is scope creep relative to the unproven core semantics.
   - It duplicates surfaces before determinism/resume/idempotency/schema stability are nailed down (`REDESIGN_PLAN.md:405` to `REDESIGN_PLAN.md:412`).

24. PR 19 (CI integration lane) should probably be earlier than REST API, and must specify alignment.
   - Plan doesn’t say whether integration lane is docker services, Nix services, or both (`REDESIGN_PLAN.md:415` to `REDESIGN_PLAN.md:423`).

25. PR 20 (Remove legacy machine) prerequisites are incomplete.
   - Beyond “no crate depends on legacy,” you need persisted formats and CLI contracts stabilized and migration docs updated, otherwise you can delete the only stable path prematurely (`REDESIGN_PLAN.md:426` to `REDESIGN_PLAN.md:437`).

## Review Checklist Critique (Missing the Dragons)

1. Add “secret handling policy (context + artifacts + errors)” to the checklist (`REDESIGN_PLAN.md:440`).
2. Add “attempt/retry semantics + idempotency enforcement” to the checklist (`REDESIGN_PLAN.md:440`).
3. Add “fast-lane persistent event store choice (sqlite vs file log) to support crash/resume” to the checklist (`REDESIGN_PLAN.md:440`).
4. Expand “canonical JSON crate choice” to include “test vectors + number constraints + rejection behavior” (`REDESIGN_PLAN.md:443`, `REDESIGN.md:69` to `REDESIGN.md:72`).

