# RFC_CONFIG implementation review

Date: 2026-07-14  
Branch reviewed: refac-config  
Review status: **not ready for full RFC compliance approval**

This document records the architecture review of the current platform against
[RFC_CONFIG.md](RFC_CONFIG.md) and [IMPL_PLAN_RFC_CONFIG.md](IMPL_PLAN_RFC_CONFIG.md).

## Executive verdict

The PostgreSQL configuration-catalog cutover is substantially implemented, but the platform is
not yet fully compliant with the RFC or its implementation-plan definition of done.

Two findings are material correctness issues:

1. EVM contract relational validation still occurs in runtime state execution instead of through
   operation-owned pure builders before graph expansion, certification, and admission.
2. The current PostgreSQL test called collect-then-report launches three independent runs. It does
   not prove one composed parent operation with collector children, readiness dependencies, and a
   report child.

The remaining findings are mandatory proof gaps, codec hardening, architecture-boundary checks,
documentation drift, and explicit decisions about known variances.

## Review evidence

Before this review document was added, the reviewed source tree was clean. The following
verification entry points passed:

- cargo fmt --all -- --check
- cargo check --workspace
- cargo test --workspace
- nix run .#model-check
- nix run .#check
- nix run .#test
- nix run .#test-db
- nix run .#ci — 14 successful CI nodes, 0 failures

Measured implementation metrics match the RFC’s stated final metrics:

| Metric | Baseline | Current | Delta |
|---|---:|---:|---:|
| Workspace packages | 53 | 52 | -1 |
| Unique direct workspace dependency pairs | 411 | 410 | -1 |
| Tracked Rust lines | 214,392 | 212,999 | -1,393 |
| Top-level public declarations | 1,754 | 1,744 | -10 |

Green gates and matching metrics demonstrate that the cutover compiles and that the old package
residue has largely been removed. They do not prove every RFC contract because several required
integration, corruption, replay, and codec tests are not present.

## Areas already implemented

The following areas are present and broadly aligned with the RFC:

- PostgreSQL catalog_values storage with exact (name, schema_id, digest) identity.
- Canonical JSON byte persistence and digest verification.
- Append-only database triggers for update, delete, and truncate rejection.
- Exact-key idempotent catalog append behavior.
- Bounded keyset catalog listing and exact export.
- Strict setup TOML import for all nine registered setup value kinds.
- ValidatedConfig<T> validation and canonicalization.
- Setup secret-field and credential-bearing URL scanning.
- Typed CatalogRef<T> validation and strict wire decoding.
- App-owned typed catalog resolution with schema checking, strict decoding, semantic validation,
  recanonicalization, and byte/digest equality verification in
  [crates/app/src/entry_point.rs](crates/app/src/entry_point.rs#L414).
- Eight exact public entry points and private exhaustive application dispatch.
- Strict CLI and REST JSON launch ingress.
- Removal of the old authored-config crates, old PostgreSQL storage crate, latest-version
  selection, old config flags, and public registry machinery.
- Runtime configuration remaining process-local and outside certified specs, events, facts,
  artifacts, public output, and replay authority.
- Typed collect-then-report operation and pure derivation tests.
- Removal of the old EVM config crate and movement of EVM action/import and entry-config types to
  their semantic owners.

## Required work

### 1. Move EVM relational validation before planning

This is the primary implementation defect.

The application currently resolves EVM contract values and constructs entry configs through
infallible constructors such as
[EvmContractConfigureEntryConfig::new](crates/ops/evm-contract-lifecycle-op/src/lib.rs#L92).
The same pattern is used for deploy, validate, and lifecycle in
[crates/app/src/entry_point.rs](crates/app/src/entry_point.rs#L231).

Source-run import/context compatibility is instead checked in runtime state code by
[validate_source_run_import_policy](crates/states/evm-contracts/src/lib.rs#L763).

Implement operation-owned pure builders that:

- Return typed errors instead of constructing invalid complete configs.
- Validate every statically knowable context/import/action relationship before Operation::expand.
- Validate required lifecycle stages.
- Validate exact context references and accepted-context policies.
- Validate network, contract identity, signer intent, action, and import joins wherever those
  relationships are represented in typed config.
- Preserve runtime evidence verification for checks that necessarily depend on an actual source
  run, but do not defer statically decidable mismatches to runtime.
- Produce stable, redacted CLI and REST errors.
- Prove through tests that invalid inputs fail before certification, admission, spec persistence,
  or state attempts.

This is required by the operation-owned construction contract in
[RFC_CONFIG.md](RFC_CONFIG.md#config-construction-stays-operation-owned).

### 2. Add a real one-parent collect-then-report parity test

The current test named
collect_then_report_completes_from_collector_written_platform_holdings performs:

1. One BTC CLI run.
2. One EVM CLI run.
3. One separate portfolio_snapshot report run.
4. Replay of all three runs.

The sequence is visible in
[tests/integration/tests/collect_then_report_native_balances.rs](tests/integration/tests/collect_then_report_native_balances.rs#L157).

It does not invoke mfm.portfolio/collect_then_report@1, does not create one parent run, and does
not prove parent-to-child lineage or readiness dependencies.

Add the required test that:

- Creates one PostgreSQL run.
- Launches the composed typed operation.
- Records BTC and EVM collector facts within that run.
- Runs the readiness bridge and report child.
- Produces the final report from the same certified graph.
- Verifies parent/child scopes and lineage.
- Uses the direct certified-program helper without shell or multi-start orchestration.
- Retains separate standalone collector and report tests.

This is required by
[IMPL_PLAN_RFC_CONFIG.md](IMPL_PLAN_RFC_CONFIG.md#commit-3-add-typed-collect-then-report-operation).

### 3. Complete catalog-backed launch coverage

Add integration coverage for all eight exact entry points:

- mfm.portfolio/portfolio_snapshot@1
- mfm.bitcoin/btc_address_balance@1
- mfm.evm/evm_native_balance@1
- mfm.evm.contract/deploy@1
- mfm.evm.contract/configure@1
- mfm.evm.contract/validate@1
- mfm.evm.contract/lifecycle@1
- mfm.portfolio/collect_then_report@1

Each entry point needs successful and failure-path coverage for:

- Successful catalog-backed preparation and launch.
- Unknown entry-point IDs.
- Unversioned IDs.
- Missing catalog rows.
- Wrong digest.
- Wrong schema/type.
- Corrupt canonical bytes.
- Semantic validation failure.
- Relational builder failure.
- Stable redacted CLI and REST errors.

The dispatch implementation exists, but the required end-to-end proof does not.

### 4. Add CLI/REST semantic parity tests

The CLI and REST paths use the same application preparation code, but the required parity proof is
missing.

Add tests that submit semantically identical requests through both surfaces and compare:

- Resolved complete config.
- Certified config material.
- Certified spec material.
- Catalog source evidence.
- Admission behavior.
- Error codes and redaction behavior.

Explicitly test rejection of:

- --op
- --op-version
- --config
- --config-format
- Old REST op, op_version, config_format, and config fields.
- TOML strings in REST requests.
- Requests missing digest.
- Unknown request fields.
- Legacy or unversioned entry-point IDs.

### 5. Complete PostgreSQL catalog test coverage

The current catalog tests cover basic append/load/list/export and immutability, but not the full
required matrix. Existing coverage is concentrated in
[crates/storages/postgres/src/run_store/tests/catalog.rs](crates/storages/postgres/src/run_store/tests/catalog.rs#L29).

Add tests for:

- Changed canonical content under the same name/schema creating a new digest row.
- Exact-key conflict with different bytes rolling back the entire batch.
- A later invalid row causing no earlier batch row to persist.
- Duplicate identities within one batch.
- Invalid names, schema IDs, digests, and payload sizes.
- Noncanonical bytes inserted directly into PostgreSQL.
- Digest mismatch inserted directly into PostgreSQL.
- Malformed, oversized, or corrupt rows on load.
- Corrupt rows during list/export.
- Keyset pagination across multiple pages.
- Page-size boundaries, including 1 and 100.
- Exact byte-identical export.
- Errors never including canonical payload bytes.
- Append-only rejection for update, delete, and truncate.

The database schema_id constraint in
[crates/storages/postgres/migrations/0001_store.sql](crates/storages/postgres/migrations/0001_store.sql#L421)
is weaker than the typed SchemaId grammar. Strengthen the database constraint or explicitly prove
that Rust-side validation is the intended authority. Also consider deterministic binary/C collation
for keyset ordering in
[crates/storages/postgres/src/catalog.rs](crates/storages/postgres/src/catalog.rs#L147).

### 6. Complete setup-import coverage

The setup implementation is present in
[crates/app/src/config_setup.rs](crates/app/src/config_setup.rs#L86), but the required test matrix
is incomplete.

Add tests for:

- Unknown nested fields in every registered setup family, not only representative families.
- Oversized setup files.
- Oversized individual values.
- Invalid UTF-8 and malformed TOML.
- Conflicting duplicate identities.
- Dry preparation proving no SQL write occurs before the entire document validates.
- Stable portfolio digest after semantically equivalent reordering.
- Export refusing an existing output path.
- Export producing byte-identical canonical JSON.
- Import output containing only (name, schema_id, digest).
- Secret-like field rejection across all nested types.
- Passwords, mnemonics, private keys, unlock paths, credential-bearing URLs, bearer tokens, and
  unknown secret-like fields.
- No rejected value or canonical payload appearing in errors.

Add setup CLI integration tests for import, list, and export; current coverage is primarily unit
level.

### 7. Add an application resolver test suite

resolve_catalog is implemented in broad accordance with the RFC, but there is no dedicated
application-level test suite covering the required failures.

Add tests for:

- Missing row.
- Wrong schema type.
- Wrong digest.
- Wrong JSON shape.
- Unknown typed fields.
- Semantic validation failure.
- Noncanonical stored bytes.
- Recanonicalized bytes differing from stored bytes.
- Digest differing from both the request and stored bytes.
- Corrupt raw storage rows.
- Redacted errors.
- Exact catalog source evidence.
- Resolver behavior for every concrete config family.

### 8. Harden launch-evidence codecs

Launch evidence creation currently sorts and deduplicates sources in
[crates/kernel/events/src/v1/mod.rs](crates/kernel/events/src/v1/mod.rs#L501).

The decoder only reads required fields and ignores unknown fields in
[crates/kernel/store/src/v1/event_codec_decode.rs](crates/kernel/store/src/v1/event_codec_decode.rs#L967).
Consequently, a payload containing current fields plus obsolete fields such as resolved_op_id or
entry_point_registry_digest can be accepted.

Implement strict object-key validation for current launch evidence and add tests for:

- Sorted source identities.
- Deduplication.
- Current codec round trips.
- Rejection of old evidence fields.
- Rejection of mixed current/legacy evidence.
- Rejection of unknown evidence fields.
- Unrelated entry-point summaries not changing another launch’s admission evidence.
- RunIdentityMaterialV1 remaining limited to certified spec hash, store scope, and invocation-key
  digest.

### 9. Prove catalog-free replay and resume

The code structure appears to keep catalog access in launch preparation, but the required
operational proof is missing.

Add PostgreSQL integration tests showing that:

- Resume does not query catalog values.
- Replay does not query catalog values.
- Status does not query catalog values.
- Stream inspection does not query catalog values.
- Public-output rendering does not query catalog values.
- Runtime config is not loaded by read-only paths.
- After admission, catalog SELECT access can be revoked while run-table access remains, and
  resume/replay still succeed.
- Deleting or changing catalog rows after admission cannot change the admitted run.

### 10. Add explicit architecture-boundary enforcement

Current metadata checks verify package shape, but the implementation plan requires stronger
source-level boundary checks.

Add architecture tests proving:

- Only app setup/resolution and operation-owned pre-planning request types depend on mfm-catalog-model.
- No CatalogRef<T> appears in kernel, runtime, replay, states, certified operation/state configs,
  event payloads, or transports.
- Storage does not import CatalogRef<T>, MfmConfig, ValidatedConfig<T>, or domain config types.
- Only app calls catalog persistence operations.
- Operation/state crates have no app, SQLx, PostgreSQL, CLI, REST, or runtime-config dependencies.
- Binaries do not implement setup decoding or domain config construction.
- The collect-then-report operation’s complete config and certified graph contain no CatalogRef<T>.
- Readiness-state placement follows the approved taxonomy.

CollectThenReportReadinessState currently lives in the operation crate and is registered from
[crates/app/src/composition.rs](crates/app/src/composition.rs#L16), while the implementation plan
calls for minimal readiness-state support in crates/states/portfolio. Either move it or record an
explicit architecture approval for the operation-local aggregation-state variance.

### 11. Resolve the runtime-configuration limitation

The RFC explicitly documents a known limitation: the app currently loads the whole runtime
configuration file, so malformed unrelated signer data can block a validation-only operation. This
is documented in [RFC_CONFIG.md](RFC_CONFIG.md#runtime-config-today).

To claim full implementation, either:

- Use selective runtime-family loading based on the operation’s actual needs; or
- Record a formal architecture decision accepting the current whole-file behavior as a deliberate
  variance and retain the regression tests.

This is a known documented limitation rather than an undocumented regression, but it prevents an
unqualified “fully compliant” claim until the variance is accepted or removed.

### 12. Update contradictory documentation

The documentation is not synchronized with the cutover.

Required updates:

- Rewrite [crates/app/README.md](crates/app/README.md#L5), which still describes entry-point
  registries, registered operation names, authored-config normalization, and a production run-store
  abstraction.
- Update [docs/architecture.md](docs/architecture.md#L42), which references the nonexistent
  docs/RFC_ENTRYPOINT_OP.md and still describes the old “typed or authored input” pipeline.
- Add the catalog/setup/exact-reference/runtime/replay boundary to
  [docs/design.md](docs/design.md#L354), since it is the authoritative design contract and
  currently has no corresponding catalog section.
- Remove or correct the undocumented MFM_SOURCE_REVISION claim in
  [bin/rest-api/README.md](bin/rest-api/README.md#L21); no implementation currently uses it.
- Clean up the duplicated CLI error documentation.
- Update [docs/portfolio-collect-then-report.md](docs/portfolio-collect-then-report.md) so it
  distinguishes independently launched workflows from the required one-parent composed workflow
  until the new parity test exists.

## Non-negotiable architecture and execution rules

This section is part of the engineer execution contract. It applies to every implementation commit
that closes the findings above.

### Principle 0: simplify the system while preserving clarity

Optimize for fewer concepts, fewer code paths, fewer public types, fewer duplicated responsibilities,
and fewer places future changes must touch. LOC reduction is valuable, but code must remain readable,
typed, and explicit.

The engineer must not introduce a new public type, trait, registry, wrapper, or service unless it
enforces an invariant that cannot live in an existing owner. In particular, do not introduce:

- A generic catalog builder trait.
- A dynamic builder registry.
- A `Resolved<T>` abstraction.
- A second catalog identity model.
- A second canonicalization authority.
- A second runtime-configuration loader beside the existing one.
- Compatibility wrappers around deleted APIs.

Remove the public `CatalogValueSource` alias in
[`crates/storages/postgres/src/catalog.rs`](crates/storages/postgres/src/catalog.rs#L66) unless a
distinct invariant is added. It currently aliases `CatalogValueKey` without adding a boundary or
validation rule.

Retain `CatalogName`, `CatalogRef<T>`, storage keys/rows, application discovery identities, and
event source evidence only when their distinct trust boundary, serialization contract, or invariant
is documented. Do not add another type for the same `(name, schema_id, digest)` identity.

Use data-driven matrices and shared fixtures where they reduce duplicated test and launch paths. A
single application semantic matrix plus thin CLI/REST transport tests is preferred over duplicating
the full live launch scenario through every transport.

### Principles 1 and 2: breaking changes and deletion are the default

This repository does not maintain backward compatibility. All breaking changes required by the RFC
are allowed and expected.

The engineer must not add or retain:

- Compatibility decoders.
- Fallback request parsing.
- Dual event codecs.
- Old and new launch paths in parallel.
- Facade crates or deprecated re-exports.
- Feature flags retaining the old implementation.
- Aliases preserving deleted public APIs.
- Dual writes or compatibility migrations for the removed launch contract.
- “Temporary” shims intended to be removed later.

When old behavior or implementation is useful during the refactor, recover it from Git history. Do
not preserve it in the working tree. Old code, old docs, old fixtures, old flags, and old public
types must be deleted rather than hidden or deprecated.

Negative tests that assert deleted names or packages remain absent are allowed. They are deletion
guards, not compatibility code. Residue searches may retain old names only in clearly labelled
historical RFC/implementation-plan text and deletion assertions.

### Single-choice architecture decisions

The engineer must not implement multiple alternatives “for safety.” Before touching an ambiguous
surface, record one decision in the existing architecture/design documentation and delete the
rejected path.

The following decisions require explicit resolution:

- EVM static relationship checks belong in operation-owned pre-planning builders; runtime retains
  only checks requiring actual source-run evidence.
- The readiness state either remains operation-local as justified composition-specific aggregation,
  or moves to the portfolio state crate. Do not keep both implementations. The simpler default is
  operation-local ownership while it has no independent reusable consumer.
- Runtime configuration either remains intentionally whole-file-loaded as the RFC-documented
  variance, or selective loading replaces it. Do not retain a whole-file fallback beside selective
  loading. The lower-complexity default is to accept the documented behavior unless a concrete
  requirement rejects it.
- PostgreSQL and Rust storage validation must have one clearly defined authority for each invariant.
  Do not create permanently divergent SQL and Rust grammars.

If an architecture decision cannot be made from the RFC, `docs/design.md`, and
`docs/architecture.md`, the engineer must stop that work item and obtain an architect-agent review
before coding.

## Progressive commit contract

Work must be committed progressively. Each commit contains one logical change, has an intentional
lower-case subject, and leaves the repository buildable and testable. Do not combine unrelated
cleanup with a behavior change merely to reduce the number of commits.

Use this sequence unless an architect-agent approves a different decomposition:

1. `fix evm contract preplanning joins`
   - Add operation-owned EVM builders and typed errors.
   - Remove infallible construction paths that permit invalid relationships.
   - Add pure builder and pre-admission regression tests.
   - Update directly affected rustdoc and architecture documentation.

2. `add composed postgres parity`
   - Add the one-parent collect-then-report PostgreSQL parity scenario.
   - Verify collector facts, readiness dependencies, report output, scopes, and lineage.
   - Retain independent collector and report behavior tests.

3. `complete catalog storage contract tests`
   - Add atomicity, conflict, corruption, bounds, pagination, export, and redaction coverage.
   - Remove the redundant `CatalogValueSource` alias.
   - Keep storage raw and domain-agnostic.

4. `complete setup and resolver contract tests`
   - Add full setup-family strictness, secret, size, duplicate, no-partial-write, export, and
     resolver failure matrices.
   - Add setup CLI integration coverage.

5. `reject legacy launch evidence and prove replay independence`
   - Make current event evidence decoding reject unknown and legacy fields.
   - Add evidence ordering/round-trip tests.
   - Add catalog-free replay, resume, status, stream, public-output, and post-admission revocation
     tests.

6. `enforce config architecture boundaries`
   - Add source-level catalog dependency and `CatalogRef<T>` boundary checks.
   - Resolve readiness-state ownership.
   - Resolve and document runtime-loading and storage-validation decisions.

7. `delete stale config documentation`
   - Delete obsolete documentation claims and references.
   - Update `docs/design.md`, `docs/architecture.md`, app/CLI/REST docs, and workflow docs in the
     same change that finalizes the contract.

8. `docs: record rfc config implementation review`
   - Commit this review document separately if it is not already committed.

For every commit, run focused checks first, then the mandatory repository gates before committing:

- `nix run .#check`
- `nix run .#test`
- `nix run .#test-db`

Run `nix run .#ci` after major work and for final merge readiness. A commit is not ready to publish
if any mandatory gate is red, skipped, or replaced by a narrower command without an explicit reason.

## Architect-agent support contract

Architect-agent assistance is part of the engineer workflow, not an optional escalation after code
has already been written.

The engineer must spawn an architect-agent before implementation when any of these are unclear:

- Whether a type or responsibility belongs in app, catalog model, storage, operation, state,
  runtime, event, CLI, or REST.
- Whether a validation belongs before planning or during runtime execution.
- Whether an existing type, function, test fixture, or code path can be deleted instead of wrapped.
- Whether a proposed change introduces a second authority, fallback, compatibility path, or public
  concept.
- Whether a commit boundary violates the one-logical-change rule.
- Whether a documentation or design decision is required before code can proceed.

The architect-agent prompt must pass these rules verbatim in substance:

- Optimize for fewer concepts, code paths, public types, duplicated responsibilities, and future
  touch points without making code cryptic.
- Do not maintain backward compatibility.
- Do not add fallbacks, shims, facades, aliases, or hidden old paths.
- Delete old code; recover it from Git history if needed.
- Review the proposed progressive commit boundary.
- Read `AGENTS.md`, `docs/code-quality.md`, `docs/architecture.md`, `docs/design.md`,
  `RFC_CONFIG.md`, `IMPL_PLAN_RFC_CONFIG.md`, and this document before advising.

The architect-agent must return:

- The invariant and ownership boundary being reviewed.
- The simplest acceptable design.
- Types, paths, and responsibilities that must be deleted.
- Required tests and verification gates.
- Any decision that must be recorded in existing architecture/design documentation.

The engineer remains responsible for implementation, tests, documentation, and gates. An
architect-agent recommendation does not authorize preserving old code or skipping verification.
If architect-agents disagree, stop the affected work, record one decision in the existing design
documentation, and proceed only with the selected design.

## Known non-gaps and intentional design choices

The following should not be treated as unresolved implementation defects:

- The remaining registry_digest references belong to kernel certification registry evidence, not
  launch-entry-point evidence.
- The old config crates and old PostgreSQL storage crate are absent.
- Old CLI/config-format residue searches are clean outside historical RFC and implementation-plan
  text.
- Runtime configuration remains a separate security boundary and is not being moved into the
  semantic catalog.
- portfolio_snapshot remains report-only.
- The catalog remains flat; nested catalog references, aliases, latest selectors, releases, and
  database-backed runtime profiles are explicitly deferred by the RFC.
- Storage intentionally remains raw and domain-agnostic; typed decoding and semantic validation
  belong to application assembly.

## Final approval condition

Do not approve the RFC implementation until the following are complete:

1. EVM contract pre-planning builders and their regression tests.
2. A true one-parent collect-then-report PostgreSQL parity test.
3. The required all-entry-point, resolver, storage, codec, replay-boundary, CLI/REST parity, and
   architecture-boundary tests.
4. Documentation updates and explicit decisions for the runtime-loading and readiness-state
   variances.
5. Removal of every redundant public type, compatibility path, fallback, facade, alias, and old
   implementation exposed by the review.
6. Re-run all residue searches, metrics, and nix run .#ci after the changes.

Until then, the implementation should be described as a substantially completed config-catalog
cutover with known correctness and proof gaps, not as fully RFC-compliant.
