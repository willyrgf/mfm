# Remove fact-query receipt signing and make collect-then-report real E2E

Status: read-only architecture audit and implementation plan.

Baseline audited: `9336b221 add public entry-point operation listing`.

This document is intentionally limited to fact-query receipt signing, its trust-root and
provisioning machinery, and the collector → fact → report end-to-end test. It does not propose
removing unrelated signatures or receipts used by manual authorization, EVM transactions, or
side-effect recovery.

## Decision

Remove the fact-query receipt signature, trust root, signing key, provisioning command, readiness
gate, and all associated public types and persisted storage.

Keep the unsigned fact-query evidence required by the report and replay contracts:

- `FactAudience::Platform` and `FactAudience::Control` visibility policy;
- the canonical query plan;
- the store read frontier and deterministic ordering/cardinality;
- returned internal fact references and returned-field summaries;
- the result-set digest;
- selection-policy evidence and selected indices;
- content-addressed `FactQueryEvidence` artifacts;
- retained descriptor and response artifacts needed to hydrate and replay selected facts.

The word “receipt” may remain for the deterministic query-result record if keeping that existing
type is the smallest clear API. It must no longer imply authorization, signing, or a separate
trust domain. No replacement signer, local MAC, process secret, or “unsigned mode” fallback is to
be introduced.

The public operation versioning work is unrelated and must remain intact. Do not remove
`OpVersion`, `--op-version`, or versioned `EntryPointOpId` as part of this change.

## Why the signature is not an authority

The current design signs a query result inside the same MFM process that:

1. connects to the Postgres store;
2. reads the authoritative run events and retained artifacts;
3. builds the query result;
4. signs the result; and
5. immediately verifies it against a public key loaded from the same store.

That signature does not establish a second principal, a separate database administrator, a
cross-process trust boundary, or a new visibility grant. A process that can use the store and the
query path can produce another valid receipt. The persisted public key is stored in that same
store, so it cannot independently prove that the store contents were authorized by someone else.

The actual semantic authority is already present:

- `FactAudience::Platform` says that a fact is visible to platform readers of the store;
- `FactAudience::Control` is restricted to control-capable internal consumers;
- `RunPrivate` facts remain private;
- committed `run_events` and retained artifacts are append-only/content-addressed authority;
- `StoreCommitOrder` and the read frontier bind the query to a deterministic store snapshot;
- the canonical plan, returned references, field summaries, result digest, and selection evidence
  make the selection reproducible;
- replay validates the retained evidence and the retained source facts/artifacts without querying
  the live store frontier or constructing live transports.

The signing layer therefore adds operational failure and secret-handling cost without adding an
independent guarantee. It also creates the exact failure seen in the CLI workflow: a collector can
successfully write a Platform fact, while the report is rejected before admission because an
unrelated key/table was not provisioned.

## Authority model after the change

### Store authority

The trusted Postgres tenancy remains the store boundary. A process with access to the same
validated schema may read Platform facts. Store connection still validates:

- the SQLx migration ledger;
- the required catalog objects;
- `store_metadata` and its store scope/epoch;
- append-only authority triggers;
- committed events, artifacts, descriptor bindings, and commit coordinates.

There is no fact-specific key table or provisioning step.

### Visibility authority

Visibility remains semantic and must not be weakened:

| Audience/scope | Allowed use after the change |
| --- | --- |
| `Platform` + default scope | Portfolio selection and public fact discovery/query |
| `Control` + default scope | Control-capable internal workflows only |
| `RunPrivate` or other non-public scope | Never exposed through public fact CLI/REST or report selection |

The public facts service must continue compiling only Platform/default plans. Public transports must
not gain an audience or scope flag that lets callers select Control or private facts.

### Query evidence

The query result remains deterministic evidence, not a permission token. A query evidence artifact
must bind:

- the canonical query plan and its hash;
- the query store scope;
- the query audience and visibility scope;
- the scope decision evidence;
- the descriptor catalog watermark;
- the committed store read frontier;
- returned fact references in deterministic order;
- returned field summaries when requested;
- result-set digest;
- exact/at-least cardinality;
- selection-policy digest, selected indices, and selected-summary digest when applicable.

The artifact itself is content-addressed and retained through the existing `ArtifactReferenced` /
`FactQueryEvidence` path. Its canonical bytes, artifact digest, event binding, and source-artifact
retention provide integrity. No signature field is needed.

## Invariants that must remain

These are acceptance criteria for the implementation, not optional test ideas.

### Fact recording and visibility

- A collector writes a `FactRecorded` event only after its descriptor and response artifacts are
  staged and bound to the certified run.
- Platform fact rows retain their source run/event coordinates, descriptor hash, response artifact
  evidence, producer provenance, visibility, and store commit order.
- The rebuildable `fact_index` and `fact_index_terms` tables remain caches, never sole semantic
  authority.
- Postgres query execution continues rebuilding candidate facts from committed `RunAdmitted` /
  `FactRecorded` events and retained descriptor/response artifacts.
- A query filters by descriptor hash, audience, visibility scope, predicates, and certified
  ordering. It must not silently broaden a Platform query to Control or private facts.
- A Platform fact is readable by any process that can access the same validated Postgres store;
  no per-process key is required.

### Deterministic query execution

- Empty query batches still return an empty result without touching query state.
- Non-empty batches execute under one `REPEATABLE READ READ ONLY` transaction.
- Multiple plans in a batch share the same store read snapshot/frontier.
- Ordering remains descriptor-defined with the existing deterministic tie breakers.
- Limits retain their existing cardinality semantics (`Exact` versus `AtLeast`).
- Returned rows remain aligned with the query evidence record; mismatched row count, row reference,
  or returned fields fail closed.
- Structural evidence validation recalculates the result-set digest and validates plan/frontier/
  scope/selection/cardinality relationships.
- The content digest of canonical `FactQueryEvidence` bytes remains the persisted identity of the
  evidence artifact.

### Replay and resume

- Replay loads a verified run history and retained artifacts before trusting any evidence.
- Replay parses and structurally validates the retained query evidence.
- Replay verifies descriptor resolution against retained descriptor authority.
- Replay verifies every returned fact reference against the retained source fact event and the
  retained descriptor/response artifact evidence.
- Cross-run source facts remain allowed only through explicit retained source fact events and
  artifacts referenced by the query evidence.
- Replay never re-queries the current fact projection/frontier.
- Replay never constructs live transports, runtime config, signer providers, or keystores.
- Missing, malformed, tampered, mismatched, or incomplete evidence still fails closed; the error
  is a structural evidence/replay error, not an authority-provisioning error.
- Resume is no longer blocked before admission merely because a fact-query key or root is absent.
  If the required facts are absent, the certified report graph fails through its normal typed fact
  selection path after admission.

### Public output and security

- Public facts still expose only descriptor-approved returnable fields.
- Public responses still redact internal references, artifact ids, hashes, run/event coordinates,
  Control rows, private rows, provider secrets, and runtime configuration.
- Removing the signing key must not cause any secret to move into an artifact, event, CLI output,
  REST response, or error message.
- EVM transaction receipts, manual-resolution signatures, and side-effect receipt evidence remain
  untouched; only fact-query receipt authentication is removed.

## Exhaustive removal and refactor inventory

The following inventory is intentionally organized by ownership boundary. Every item is either
deleted, simplified, updated at its callers, or explicitly retained. A final repository search
must find no remaining fact-receipt authority term outside historical documentation that is also
being deleted.

### 1. Facts kernel contract

#### `crates/kernel/facts/src/receipt.rs`

Remove:

- `StoreReceiptAuthenticationScheme` and its tag implementation;
- `StoreReceiptAuthentication` and all constructors/accessors;
- `FactQueryReceipt.store_receipt_authentication`;
- `FactQueryReceipt.store_receipt_hash` if the implementation removes the now-redundant inner
  body hash;
- `FactQueryReceipt::store_receipt_authentication()`;
- the authentication argument from `FactQueryReceipt::from_parts`;
- `FactQueryReceiptMaterial` as a separate public type. It exists only to separate unsigned
  material from the later signing step.

Refactor:

- Build the unsigned deterministic receipt directly from rows/frontier/ordering/cardinality.
- Keep `FactQueryReceipt` only as query-result evidence, with no authorization semantics.
- Prefer one constructor that computes returned refs, field summaries, result-set digest, and
  cardinality. Do not replace the material type with another intermediate signer/material type.
- If the inner body digest is retained for structural self-validation, rename it to a neutral
  `receipt_body_hash` and document that it is a consistency digest, never an authority token. The
  smaller preferred design removes it because the canonical evidence artifact digest already binds
  the whole record and structural validation can recompute the result-set digest.

Retain:

- `DescriptorCatalogWatermark`;
- `StoreCommitOrder`;
- `StoreReadFrontier` and `StoreReadFrontierType`;
- `ReturnedFactFieldSummary` / `ReturnedFieldSummaries`;
- `QueryResultCardinality`;
- `FactQueryReceipt`, `FactQueryResult`, `FactQueryResultRow`, and row-alignment validation;
- `FactSelectionEvidence` and `FactQueryEvidence`.

#### `crates/kernel/facts/src/codec.rs`

Remove:

- `parse_store_receipt_authentication`;
- `canonical_store_receipt_authentication_value`;
- authentication imports and parsing of `store_receipt_authentication`;
- canonical `store_receipt_hash` / authentication fields if the unsigned shape is flattened;
- receipt-body hashing helpers and `FactQueryReceiptBodyParts` if the inner hash is removed;
- comments saying that store authentication is deferred to `mfm-store`.

Refactor:

- Parse and canonicalize one unsigned receipt shape containing the frontier, returned refs,
  summaries, result digest, and cardinality.
- Keep canonical plan-hash, scope, frontier, result-set, and selection validation.
- Change the persisted fact-query evidence contract so old signed bytes are rejected. Do not add a
  parser that accepts both shapes. The preferred contract change is a new fact-query evidence
  schema/version for the unsigned shape, while leaving the unrelated fact-plan and descriptor
  contracts unchanged.
- Update all canonical JSON and digest fixtures to the new shape.

#### `crates/kernel/facts/src/ids.rs` and `src/lib.rs`

Delete the now-unused public `StoreIdentity` and `StoreKeyId` checked-string types and their
exports. They are exclusively used by the fact-query receipt authentication design in the audited
baseline.

Keep `StoreScopeRef`; it is query scope evidence and is unrelated to a signing identity.

#### `crates/kernel/facts/src/tests.rs`

Delete tests that assert Ed25519 scheme/key/signature requirements, authentication fields, or
authentication-excluded body hashes. Replace fixture construction with unsigned receipts and keep
tests for:

- deterministic canonical query/evidence bytes;
- plan/frontier scope mismatch rejection;
- result-set digest mismatch rejection;
- row/receipt alignment;
- cardinality and limit behavior;
- sorted/unique selection indices;
- tampered canonical evidence rejection;
- returned-field summary validation.

### 2. Kernel store and test fixtures

#### Delete `crates/kernel/store/src/v1/receipt_authentication.rs`

Delete the entire module, including:

- `FactQueryReceiptTrustRoot`;
- `fact_query_receipt_authentication_message`;
- `verify_fact_query_receipt_authentication`;
- `validate_fact_query_evidence_recording`;
- Ed25519 verification code and all authentication-only unit tests.

The facts kernel owns structural evidence validation after this change. The store must not be a
second receipt-authentication service.

#### `crates/kernel/store/src/v1/mod.rs`

Remove:

- `StoreError::ReceiptAuthentication`;
- the `receipt_authentication` module declaration;
- its public re-exports.

Audit all `StoreError` conversions so database/corruption/codec errors remain typed and do not
reuse a deleted authority error code.

#### `crates/kernel/store/src/v1/test_support.rs`

Delete:

- `fact_query_receipt_trust_root_for_test`;
- `SignedFactQueryReceiptFixtureInputForTest`;
- `signed_fact_query_receipt_for_test`;
- `signed_fact_query_receipt_for_projection_for_test`;
- signing-key, store-identity, key-id, signing-message, and authentication fixture code.

Keep one small unsigned projection-query fixture helper that builds the canonical receipt from a
projection/frontier. It must not carry a fake root or fake signer merely to satisfy runtime tests.

Remove the `ed25519-dalek` dependency from `crates/kernel/store/Cargo.toml` if no unrelated store
code uses it.

### 3. Postgres storage

#### `crates/storages/stream-store-postgres/migrations/0001_run_store.sql`

Delete the `fact_receipt_trust_root` table and all of its:

- singleton row shape;
- store identity, scheme, key-id, and verifying-key columns;
- constraints;
- `fact_receipt_trust_root_no_update` trigger.

Remove the table from every schema contract list in `schema.rs` and from the immutable-table list.

This is a destructive pre-production baseline replacement. Do not create a compatibility migration,
dual schema reader, automatic conversion, or fallback. Existing databases/schemas containing the
old migration checksum or signed evidence must be reset before they are used with the new code.

Because the store contract changed, update the baseline schema contract version consistently in the
SQL insert and validation code if the implementation uses the version to distinguish the new
baseline. Do not accept both old and new contract versions.

#### `crates/storages/stream-store-postgres/src/schema.rs`

Delete:

- `PostgresSchema::provision_fact_receipt_trust_root`;
- root-loading and root-parsing code;
- Ed25519/store-identity/key-id imports;
- zeroizing signing-key imports used only by provisioning;
- `PostgresStoreAuthority` root field/accessor;
- `PostgresStoreAuthorityError::FactReceiptTrustRoot`;
- required table/trigger/constraint/immutable entries for the root.

Refactor `validate_pool` and `validate_store_metadata` so the only store-specific authority they
load for this concern is the validated store scope/epoch and the append-only/catalog contract.

#### `crates/storages/stream-store-postgres/src/run_store/mod.rs`

Delete:

- `fact_receipt_signer` from `PostgresRunStore`;
- `connect_with_fact_receipt_signer`;
- `connect_with_fact_receipt_signing_key_bytes`;
- `fact_receipt_queries_ready`;
- `PostgresFactReceiptSigner` export and imports;
- signer-matching and signer-loading paths.

`PostgresRunStore::connect` becomes the only production store connection path for this concern.

#### Delete `crates/storages/stream-store-postgres/src/run_store/receipt_authentication.rs`

Delete the signer, trust-root comparison, missing-signer errors, and all authentication-only
helpers. Remove the module declaration and public export from `src/lib.rs`.

Remove `ed25519-dalek` and `zeroize` from this crate manifest only if no remaining storage code
uses them. Do not remove `zeroize` from unrelated keystore crates.

#### `crates/storages/stream-store-postgres/src/run_store/fact_queries.rs`

Retain the authoritative query implementation and simplify its signatures:

- `execute_fact_queries` no longer requires a signer or trust root;
- `execute_fact_queries_client` takes only the pool and plans;
- `execute_fact_query_tx` takes only the transaction, plan, and authoritative events;
- `build_fact_query_receipt_tx` builds an unsigned receipt;
- the repeatable-read transaction remains unchanged;
- authoritative event/artifact rebuild remains unchanged;
- audience/scope filtering and deterministic ordering remain unchanged;
- remove post-build signature verification.

#### `crates/storages/stream-store-postgres/src/run_store/tests.rs` and `src/lib.rs`

Delete root insertion helpers, signer fixtures, signer mismatch tests, readiness assertions, and
the schema provisioning lifecycle test. Keep Postgres parity tests for:

- authoritative event/artifact rebuild;
- Platform/Control filtering;
- deterministic query ordering and frontiers;
- structural evidence and retained artifact behavior;
- schema/catalog validation and append-only protection.

Update the public crate export list so it no longer exposes a fact-receipt signer.

### 4. Runtime and replay plumbing

#### `crates/kernel/runtime/src/invocation.rs`

Remove the trust-root field from `ErasedRunCtx`, remove the argument from
`ErasedRunCtx::from_prepared`, and delete the private accessor. A runner context must not carry
process/store signing authority.

#### `crates/kernel/runtime/src/scheduler.rs`

Remove:

- `SerialTypedScheduler.fact_query_receipt_trust_root`;
- `with_fact_query_receipt_trust_root`;
- trust-root cloning into ordinary/framework lifecycles.

The scheduler still receives the certified runner registry and artifact store; that is sufficient
for the existing runtime authority model.

#### `crates/kernel/runtime/src/attempt.rs` and `src/framework_lifecycle.rs`

Remove trust-root fields, constructor parameters, and context forwarding. Keep all ordinary and
framework attempt lifecycle, failure-safe terminalization, artifact staging, and retention rules.

#### `crates/kernel/runtime/src/runner_kit.rs`

In `record_fact_query_evidence`:

- delete the missing-root error;
- call `mfm_facts::validate_fact_query_evidence` directly;
- keep artifact construction, `FactQueryEvidence` role binding, returned-ref authority retention,
  descriptor/response retention, and staged-artifact validation.

Update its test fixtures to use unsigned receipts. The `FactQueryEvidence` artifact remains
private replay evidence and must not become a public output shortcut.

#### `crates/kernel/runtime/src/tests.rs` and `Cargo.toml`

Delete test signing keys, trust-root fixtures, signed receipt fixture inputs, and context setup
that exists only to supply them. Remove the runtime crate's `ed25519-dalek` dependency if its only
remaining uses are these fixtures. Keep runtime tests for artifact role contracts, retention
authority, returned-fact descriptor/response artifacts, and replay evidence.

#### `crates/kernel/replay/src/lib.rs`

Remove the private `fact_query_receipt_trust_root` field from `ReplayReadAuthority` and all
constructor variants whose names include `with_fact_query_receipt_trust_root`.

Collapse the constructor family to the smallest set that still supports:

- a verified run history;
- optional retained cross-run source fact events;
- optional additional certified artifacts.

In `verify_fact_query_evidence_reference`:

- delete the missing-root failure;
- parse canonical evidence;
- call `mfm_facts::validate_fact_query_evidence`;
- continue verifying descriptor resolution and every returned fact reference against retained
  source fact/artifact authority.

Do not replace the root with a boolean, an “authority ready” flag, or a permissive fallback.

#### `crates/kernel/replay/src/v1/tests.rs` and UI fixtures

Delete missing-root and wrong-root cases, trust-root injection, signing keys, and signed fixture
builders. Retain and strengthen cases for:

- missing source facts;
- mismatched returned references;
- invalid visibility;
- descriptor/response artifact mismatch;
- tampered canonical evidence;
- missing retained evidence.

Update `crates/kernel/replay/tests/ui/fail/replay_read_authority_fields_private.stderr` after the
private field is removed. Remove the replay crate's Ed25519 dev dependency if it has no unrelated
manual-auth use.

### 5. Application assembly and fact providers

#### `crates/app/src/lib.rs`

Delete:

- `MFM_FACT_RECEIPT_SIGNING_KEY_FILE`;
- `PRODUCTION_FACT_RECEIPT_STORE_IDENTITY`;
- `PRODUCTION_FACT_RECEIPT_KEY_ID`;
- signing-key file discovery, decoding, zeroization, and redacted key errors;
- `provision_production_fact_receipt_authority`;
- signer/root store error mapping;
- `fact_receipt_authority_unavailable_error`;
- `runtime_spec_requires_fact_index` and all pre-admission fact-authority gates.

Refactor:

- `connect_production_run_store` directly returns `ProductionRunStore::connect`;
- `make_run_services` and `make_run_read_services` accept only their actual store/artifact/
  certification dependencies;
- `RunServices` and `RunReadServices` drop root/readiness fields;
- replay helpers accept verified history plus retained source facts/artifacts, not a trust root;
- `PublicFactQueryExecutor` maps structural/database query errors without converting them to a
  deleted authority-unavailable code.

Remove app-level `zeroize` and `ed25519-dalek` dependencies if they are used only by this feature.
Keep `Path` for runtime-config paths and keep unrelated keystore/crypto dependencies elsewhere.

#### `crates/app/src/fact_index.rs`

Production Postgres fact reads continue to use `PostgresRunStore::execute_fact_queries`.

For `ProjectionFactIndexProvider`:

- remove the fixed Ed25519 key, store identity, key id, and `receipt_trust_root()`;
- build unsigned receipts from the projection/frontier;
- retain row-count instrumentation used by tests;
- retain Platform/Control projection filtering.

#### `crates/app/src/public_facts/service.rs`

Keep the public service's Platform/default scope guard and descriptor-approved DTO conversion.
Remove only the mapping that translates a deleted `ReceiptAuthentication` store error into
`FactReceiptAuthorityUnavailable`.

#### `crates/app/src/tests.rs`, `src/entry_points.rs`, `src/evm_contracts/tests.rs`, and
`tests/manual_resolution.rs`

Update all constructor call sites to the simplified service constructors. Delete tests whose only
purpose is to assert missing/provisioned/mismatched fact receipt authority. Replace the pre-admission
authority expectation with coverage that a report is admitted and then follows normal missing-fact
behavior when no Platform fact exists. Keep runner-binding, certification, replay, and unrelated
manual-resolution tests.

### 6. Domain fixtures and adapter tests

Update unsigned receipt construction in:

- `crates/states/btc/src/lib.rs` test module;
- `crates/adapters/btc-jsonrpc/src/tests.rs`;
- `crates/adapters/portfolio/src/tests.rs`;
- `crates/kernel/runtime/src/runner_kit.rs` test module;
- all other callers found by searching for `signed_fact_query_receipt` and
  `StoreReceiptAuthentication`.

Remove `StoreIdentity`/`StoreKeyId` imports and Ed25519 test dependencies from these crates when
the search confirms there is no unrelated use. Do not change the live BTC/EVM transport receipts
or capability contracts.

### 7. REST API

#### `bin/rest-api/src/lib.rs`

Remove from `AppState`:

- `fact_query_receipt_trust_root`;
- `fact_query_authority_ready`.

Remove the corresponding production-state loading and service-constructor arguments. Delete
`require_signed_query_role`. Public facts are read-only Platform data, so `/v1/facts/:kind` and
`/v1/facts/:kind/latest` must work in the evidence-only `read` role as well as the live role.

Keep `require_live_role` for start/resume/manual-resolution routes. `/v1/ready` continues checking
the run store, not fact-receipt key state.

#### `bin/rest-api/src/tests.rs`

Update every `AppState` fixture. Rename the read-role test so it asserts that read role refuses live
run control but can serve a valid Platform fact query. Keep tests that reject Control/private
public references, malformed query shapes, and runtime-config misuse.

#### `bin/rest-api/README.md`

Delete all references to:

- `MFM_FACT_RECEIPT_SIGNING_KEY_FILE`;
- live-only signed queries;
- trust-root validation;
- provisioning;
- fact-receipt readiness.

Document that public fact reads use Platform/default visibility and store-backed evidence, and that
the read role may serve those read-only endpoints.

### 8. CLI

#### `bin/cli/src/commands/facts.rs`

Delete the `ProvisionAuthority` subcommand, `ProvisionAuthorityArgs`,
`ProvisionAuthorityOutput`, its display implementation, dispatch arm, and executor.

Do not replace it with a no-op command, deprecation message, compatibility alias, or hidden
bootstrap path.

#### `bin/cli/tests/status_contract_postgres.rs`

Delete temporary signing-key files, environment restoration, `facts provision-authority` calls,
and signer-specific CLI helpers. Keep this test focused on status/stream history. Its report resume
must now exercise the normal missing-Platform-fact path rather than missing receipt authority.

#### `bin/cli/README.md`

Delete the five signing/provisioning/authority error codes:

- `MissingFactReceiptSigningKey`;
- `FactReceiptSigningKeyReadFailed`;
- `FactReceiptSigningKeyInvalid`;
- `FactReceiptSignerInvalid`;
- `FactReceiptAuthorityUnavailable`.

Delete the environment-variable section, provisioning recipe, and signed-query wording. State that
fact commands are read-only Platform/default queries backed by the validated Postgres store.

Keep `zeroize` in the CLI if keystore commands still use it; this removal must not weaken keystore
secret handling.

### 9. Integration tests and test support

#### `tests/integration/src/test_support.rs`

Remove root/readiness construction from `in_memory_rest_app_state`. Keep the shared unsigned
`fact_query_evidences` loader and Platform/Control fixture helpers.

Extract the already duplicated Postgres test plumbing into one small shared support surface if it
is needed by more than one parity test:

- unique schema name;
- create/drop schema;
- schema-scoped database URL;
- migrate/connect retry.

Do not create a second collector→fact→report workflow helper.

#### `tests/integration/tests/collector_workflow_happy_path.rs`

Update service construction to remove trust-root arguments and signed fixture setup. Preserve its
collector interruption/resume and retained query-evidence assertions.

#### `tests/integration/tests/portfolio_snapshot_from_admitted_facts.rs`

Update service construction and receipt fixtures. Preserve the report selection, Platform fact,
retained response, and replay assertions.

#### `tests/integration/tests/parity_rest_api_postgres_smoke.rs`

Keep the small REST/Postgres readiness smoke test. Delete the entire
`postgres_fact_receipt_authority_covers_provisioning_and_admission` test and its signing-key
environment helper; it tests a concept that no longer exists and must not be replaced by a second
large workflow test.

#### `tests/integration/tests/collect_then_report_native_balances.rs`

This is the test that must become the real workflow. The detailed plan is below. Do not create a
parallel “real coverage” test with copied assertions.

### 10. Dependency and repository cleanup

After the source changes, remove only dependencies made dead by this feature:

- `ed25519-dalek` from `crates/app`, `crates/kernel/store`, `crates/kernel/runtime`,
  `crates/kernel/replay` dev code, `crates/adapters/btc-jsonrpc` tests,
  `crates/adapters/portfolio` tests, Postgres storage, and integration test support where no
  unrelated code remains;
- `zeroize` from `crates/app` and Postgres storage only if the feature was their sole use;
- generated `Cargo.lock` entries only through Cargo after the manifest cleanup.

Do not remove dependencies from `crates/core`, `crates/signers/keystore`, CLI keystore code, manual
authorization, or any unrelated cryptographic surface.

## In-place real E2E plan

The existing target test currently uses:

- `AsyncInMemoryRunStore`;
- `ProjectionFactIndexProvider`;
- direct `RunServices` construction;
- direct app launch helpers;
- a mock provider/in-memory process boundary.

That proves composition, but it does not prove the operator path that failed. The same file must be
converted so one test exercises the production Postgres store and the actual CLI binary, while
retaining the current output assertions.

### Test boundary

Keep the test at:

`tests/integration/tests/collect_then_report_native_balances.rs`

Do not add a second test file or move the workflow assertions to a new package. The architect audit
noted that the CLI package already has `cargo_bin("mfm_cli")` support; for this requested in-place
test, make the integration target use the same explicit binary contract:

- add `assert_cmd` as an integration-test dev dependency if needed;
- build `mfm_cli` explicitly before this parity target (`cargo build -p mfm --bin mfm_cli`);
- invoke the already-built binary with `assert_cmd::Command::cargo_bin("mfm_cli")`;
- do not spawn nested Cargo/Nix commands from the test;
- do not fall back to direct app services if the CLI binary is unavailable.

This keeps the test in the requested package while making the production CLI boundary explicit.

### Test setup

1. Require `DATABASE_URL`, as the other Postgres parity tests do.
2. Create a unique schema and derive a schema-scoped URL.
3. Apply the current Postgres baseline to that schema.
4. Start the existing `start_collectors_rpc_mock()` server.
5. Reuse `write_collectors_runtime_config_for_test()` for the collector runtime routes.
6. Write the existing BTC, EVM, and dual-mainnet portfolio JSON fixtures to temporary files.
7. Ensure the child process has no fact-receipt signing-key requirement. No key file, root row, or
   provisioning command should be created.

Use the shared Postgres schema helpers rather than copying the helper implementations from
`parity_rest_api_postgres_smoke.rs`.

### CLI sequence

Run the actual packaged CLI three times against the same schema-scoped store:

```text
mfm_cli --output-format json run start \
  --op btc_address_balance \
  --config <btc-config> --config-format json \
  --runtime-config <collector-runtime> \
  --database-url <scoped-url>

mfm_cli --output-format json run start \
  --op evm_native_balance \
  --config <evm-config> --config-format json \
  --runtime-config <collector-runtime> \
  --database-url <scoped-url>

mfm_cli --output-format json run start \
  --op portfolio_snapshot \
  --config <portfolio-config> --config-format json \
  --database-url <scoped-url>
```

The report deliberately omits `--runtime-config`. This proves that `portfolio_snapshot` is
report-only and reads the collector-written Platform facts instead of constructing live chain
providers.

For each command:

- assert the JSON envelope is successful;
- extract the returned run id;
- assert the run is completed;
- fail with stderr and the safe run diagnostics if it is not completed.

The report must complete without a signing key or provisioning step. This is the regression that
the current in-memory test cannot detect.

### Assertions to retain, not duplicate

Keep the existing pure assertion helpers in the same file:

- Platform holding-kind counts;
- admitted adapter factory identities;
- public snapshot discovery;
- BTC raw amount, decimals, normalized amount, and USD value;
- ETH raw amount, decimals, normalized amount, and USD value;
- USD assets and net totals;
- no-live-provider/report-only behavior.

Adapt their store boundary only:

- load the Postgres projection through `PostgresRunStore::fact_projection_snapshot()` for the
  Platform fact assertions;
- load each run stream from `PostgresRunStore` for admitted adapter identity assertions;
- parse the report JSON from the CLI response for the existing portfolio output assertions.

Do not recreate the portfolio assertions in a second file or introduce a second expected-output
fixture.

### Replay assertions

After all three runs complete, invoke the actual CLI replay command for each run:

```text
mfm_cli --output-format json run replay <btc-run-id> --database-url <scoped-url>
mfm_cli --output-format json run replay <evm-run-id> --database-url <scoped-url>
mfm_cli --output-format json run replay <report-run-id> --database-url <scoped-url>
```

Assert success and a positive retained-artifact count. This verifies that replay consumes the
recorded unsigned fact-query evidence and retained source artifacts, not a live query or a process
signer.

### What to delete from the current test file

The current third test's direct in-memory production service path becomes dead and must be deleted:

- `production_collect_then_report_services`;
- direct `RunServices::launch_prepared_entry_point_run` for the report;
- direct in-memory store setup for the collect/report scenario;
- trust-root construction and authority-ready arguments.

The first two focused in-memory collector tests may remain as fast, narrow provider/collector
tests if they continue to provide unique coverage. They are not a substitute for the parity test.
If the implementation finds that their mock providers and service builders are only supporting
duplicated assertions, delete those tests and their mock-provider code; do not preserve dead
fixtures for compatibility.

### Nixfied execution

Make the parity task that runs this target:

- require Postgres;
- build `mfm_cli` explicitly in the shared target directory;
- run the target with `--features parity-tests` and `--nocapture`;
- use the same hermetic `DATABASE_URL` and `CARGO_TARGET_DIR` as the other parity tasks.

Prefer extending the existing Postgres integration task with this target rather than introducing a
second task that duplicates service setup. If the task becomes too opaque, add one narrowly named
parity leaf and make it part of `test-db` and `ci`.

## Progressive implementation commits

Each commit must compile and pass the required gates. Do not commit an intermediate state that
leaves the workspace knowingly broken across crate boundaries.

### Commit 1: unsigned facts evidence contract

Suggested subject:

`remove fact receipt authentication from the facts contract`

Scope:

- facts receipt/codec/lib/ids;
- kernel store authentication module and test fixtures;
- unsigned canonical evidence shape/version;
- facts/store structural tests;
- direct baseline migration/schema contract edits where required.

Required checks before commit:

```bash
nix run .#check
nix run .#test
nix run .#test-db
```

### Commit 2: store, runtime, replay, and app authority removal

Suggested subject:

`remove fact receipt authority from store and runtime`

Scope:

- Postgres store/query path;
- app store/services/fact-index;
- runtime context/scheduler/runner kit;
- replay authority;
- all corresponding unit and crate tests.

Required checks before commit:

```bash
nix run .#check
nix run .#test
nix run .#test-db
```

### Commit 3: CLI, REST, documentation, and existing parity cleanup

Suggested subject:

`remove fact receipt provisioning from cli and rest`

Scope:

- delete `facts provision-authority`;
- remove signing-key env/error docs;
- allow read-only public fact queries in the REST read role;
- remove the obsolete authority lifecycle test;
- update status and REST tests;
- update persisted-surface and portfolio workflow docs.

Required checks before commit:

```bash
nix run .#check
nix run .#test
nix run .#test-db
```

### Commit 4: real collector → fact → report E2E

Suggested subject:

`make collect then report a postgres cli e2e`

Scope:

- transform `collect_then_report_native_balances.rs` in place;
- add only the minimal CLI invocation/test dependency/build support;
- reuse the existing RPC mock and output assertions;
- delete the old direct in-memory collect/report path;
- wire the target into the Postgres test composite.

Required checks before commit:

```bash
nix run .#check
nix run .#test
nix run .#test-db
nix run .#ci
```

## Final repository search and verification

After the last commit, these feature-specific terms must be absent from active code, tests, docs,
and migrations:

```text
MFM_FACT_RECEIPT_SIGNING_KEY_FILE
PRODUCTION_FACT_RECEIPT_STORE_IDENTITY
PRODUCTION_FACT_RECEIPT_KEY_ID
provision_production_fact_receipt_authority
provision_fact_receipt_trust_root
fact_receipt_trust_root
fact_receipt_queries_ready
fact_query_authority_ready
FactQueryReceiptTrustRoot
PostgresFactReceiptSigner
StoreReceiptAuthentication
StoreReceiptAuthenticationScheme
ReceiptAuthentication
signed_fact_query_receipt
SignedFactQueryReceipt
FactReceiptAuthorityUnavailable
FactReceiptSignerInvalid
MissingFactReceiptSigningKey
facts provision-authority
```

The search must be reviewed manually so unrelated matches such as EVM transaction receipts,
manual-resolution signatures, and side-effect receipt evidence are not removed accidentally.

Run focused checks in addition to the mandatory gates:

```bash
cargo fmt --all -- --check
cargo test -p mfm-facts
cargo test -p mfm-store
cargo test -p mfm-runtime
cargo test -p mfm-replay
cargo test -p mfm-stream-store-postgres --features parity-tests
cargo test -p mfm-integration-tests --features parity-tests \
  --test collect_then_report_native_balances -- --nocapture
cargo test -p mfm --features parity-tests \
  --test status_contract_postgres -- --nocapture
```

The final E2E must prove all of the following in one workflow:

```text
real CLI BTC collector
  → Postgres FactRecorded Platform fact
real CLI EVM collector
  → Postgres FactRecorded Platform fact
real CLI report with no runtime config and no signing key
  → deterministic Platform selection + public portfolio output
real CLI replay for all three runs
  → retained unsigned query evidence + retained source artifacts
```

