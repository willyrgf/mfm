# Implementation plan: typed PostgreSQL configuration catalog

Status: historical architecture record; superseded for the portfolio source-model cutover

Date: 2026-07-14

Planning baseline: the commit containing this revised plan and RFC; record its exact SHA before
implementation commit 1.

## Objective

Replace direct per-operation TOML/JSON authoring with one catalog-backed launch path:

```text
setup TOML
  -> strict typed import
  -> validated canonical MfmConfig values
  -> append-only PostgreSQL catalog

exact entry-point id + JSON request with exact CatalogRef<T> values
  -> application derives expected schemas and asks PostgreSQL for exact raw rows
  -> PostgreSQL verifies canonical-byte and digest integrity
  -> application strict-decodes, validates, and recanonicalizes typed values
  -> operation-owned pure assembly when more than one value is needed
  -> complete MfmConfig
  -> existing expansion, certification, admission, runtime, resume, and replay
```

The work is complete only when the old authored-config path and all of its crates, flags, schemas,
tests, examples, aliases, and fallback behavior have been deleted.

## Non-negotiable execution rules

Every engineering agent working from this plan must follow these rules:

1. Read `AGENTS.md`, `docs/code-quality.md`, `docs/architecture.md`, and `docs/design.md` before
   editing.
2. Optimize for fewer concepts, public types, code paths, duplicate validations, and future edit
   sites. Do not reduce line count by making code cryptic.
3. Preserve no backward compatibility. Do not add legacy decoders, deprecated aliases, dual writes,
   fallback parsing, old CLI flags, compatibility migrations, or transitional public APIs.
4. Delete superseded code in the cutover commit. Git history is the recovery mechanism.
5. Keep operations deterministic, states free of ambient IO, binaries thin, and PostgreSQL access in
   storage/application assembly.
6. Never persist or print secrets. Catalog errors and CLI/REST responses must be redaction-safe.
7. Keep each commit to one reviewable outcome. Before every commit run:

   ```text
   nix run .#check
   nix run .#test
   nix run .#test-db
   ```

8. Use lower-case commit subjects. Run `nix run .#ci` after the final implementation commit.
9. If an implementation detail would change the boundaries fixed by the RFC, stop and obtain an
   architecture decision. Do not hide uncertainty behind an abstraction.

## Definition of done

The final tree must satisfy all of the following:

- One PostgreSQL authority stores streams, artifacts, facts, and catalog values through one pool and
  one migrator.
- One `catalog_values` table stores immutable canonical bytes keyed by name, schema id, and digest.
- Every run request uses one exact entry-point id and exact catalog references. There is no `latest`
  or name-only resolution.
- CLI run start and REST run start share JSON request semantics.
- TOML is accepted for setup import and existing runtime configuration, not operation launch.
- Catalog access ends before `Operation::expand`.
- Complete generated configs and certified specs remain runtime/replay authority.
- Launch evidence contains exact entry-point id and sorted catalog source identities, not a global
  registry digest.
- `portfolio_snapshot` remains report-only and independently launchable.
- A new collect-then-report operation composes collector and report operations in one certified run.
- `crates/authored-config`, `crates/portfolio-config`, and `crates/evm-contract-config` do not exist.
- `mfm-runtime-config` remains the isolated runtime/security config boundary.
- Old entry-point registry abstractions, accepted formats, latest resolution, direct config ingress,
  old examples, and compatibility tests do not exist.
- The final workspace has 52 packages unless an approved architecture decision changes the crate
  budget.
- Final `nix run .#ci` passes, and measured package, direct workspace dependency, Rust/source LOC,
  and public-declaration deltas are reported.

## Target ownership and dependency shape

### `mfm-values`

Keep `MfmConfig` as the single persisted semantic-config contract. Add canonical serialization to
`ValidatedConfig<T>` so program certification and catalog publication use the same implementation.
Do not create another validation or canonicalization trait.

Conceptual API:

```rust
impl<T: MfmConfig> ValidatedConfig<T> {
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes, ConfigError>;
}
```

Update the program config-binding path to call this method. Catalog insertion must use the same
method after `ValidatedConfig::new`.

### New `mfm-catalog-model` crate

Create `crates/catalog-model` as package `mfm-catalog-model` with category `domain-model`. It owns
exact typed catalog input identity only; it has no SQLx, app, storage, runtime, operation, transport,
format-parsing, setup-import, or service dependency.

The public surface should be limited to:

- `CatalogName`;
- `CatalogRef<T: MfmConfig>`;
- one typed construction/decoding error if an existing MFM id error cannot represent it cleanly.

`CatalogRef<T>` contains a validated catalog name, `ContentDigest`, and `PhantomData<T>`. Its JSON
wire form is exactly:

```json
{
  "name": "portfolios/treasury",
  "digest": "<content digest>"
}
```

The request type supplies `T`, so the wire form does not repeat `schema_id`. The app resolver derives
the expected schema id from `T`. Language-neutral evidence and catalog listing do include schema id.

Catalog names use one grammar in Rust and PostgreSQL:

- 1 through 256 ASCII bytes;
- slash-separated non-empty segments;
- each segment begins with lower-case ASCII alphanumeric;
- remaining characters are lower-case ASCII alphanumeric, `.`, `_`, or `-`;
- no empty, `.` or `..` segments.

Do not add `ConfigScope`, `Resolved<T>`, `CatalogValue`, an erased catalog identity, catalog handles,
aliases, releases, format parsing, database records, a setup enum, a builder registry, or a storage
trait. `CatalogSourceEvidence` remains an event-owned checked scalar structure rather than reusing
an erased catalog-model type.

### Renamed PostgreSQL storage crate

Rename:

```text
crates/storages/stream-store-postgres -> crates/storages/postgres
mfm-stream-store-postgres             -> mfm-storage-postgres
PostgresRunStore                      -> PostgresStore
```

Delete the old package/type names and update all consumers. Do not leave re-exports or type aliases.
Internal `run_store` modules may retain run-specific names because they still own run-store logic.
Delete app aliases such as `ProductionRunStore` and `ProductionPostgresSchema` rather than replacing
them with `ProductionStore`. App may expose the concrete `PostgresStore` name or narrow constructors
and service aliases that enforce an app-level invariant.

The renamed crate owns catalog SQL and extends the existing `PostgresStoreError`. Its API is raw and
concrete: exact row-key lookup, heterogeneous row append, bounded metadata listing, and exact raw
export. Introduce at most the minimal storage-owned raw key/row types needed to carry `name`,
`SchemaId`, `ContentDigest`, and canonical bytes across the app/storage boundary. Do not introduce
repository, DAO, reader, writer, service, and transaction wrapper layers around the same table.

The storage crate must not depend on `mfm-catalog-model`, import `CatalogRef<T>`, `MfmConfig`, or
`ValidatedConfig<T>`, or decode domain types. It validates row bounds and catalog-name grammar,
parses canonical JSON, recomputes digests, and compares exact bytes because those are raw persistence
integrity responsibilities permitted by the storage dependency contract.

### Domain and operation crates

Domain crates continue to own concrete config types. Operation crates own request types and pure
assembly functions only when assembly is required. An exact reference to an already-complete
`MfmConfig` is loaded and used directly; do not add identity builders.

No operation crate may depend on SQLx, the PostgreSQL crate, `mfm-app`, runtime config, environment
variables, or filesystem APIs.

### Application

Application assembly owns exact entry-point selection, strict request decoding, typed publication
preparation, typed catalog resolution, calling pure builders, certification, and admission
preparation.

The app resolver accepts `CatalogRef<T>`, derives `T::schema_id()`, asks `PostgresStore` for one exact
raw row, strict-decodes `T`, validates it through `ValidatedConfig::new`, recanonicalizes it through
the shared config path, and requires byte equality. It returns ordinary `T`; it does not introduce a
public `Resolved<T>` or generic catalog service trait.

Setup publication validates and canonicalizes every typed value, enforces payload bounds, performs
the defense-in-depth secret-field scan, derives schema id and digest, creates the typed reference,
and converts the result to the storage-owned raw row. All preparation finishes before app invokes
batch append; storage then opens its transaction and independently rechecks raw canonical and digest
integrity.

Replace the dynamic public registry with one private `EntryPoint` enum and exhaustive methods for:

- parsing an exact id;
- returning its exact id;
- returning its request schema id;
- preparing its typed launch.

Expose only a narrow `EntryPointSummary` list for CLI discovery. Do not expose registry
construction, registration, dynamic operation traits, latest resolution, or accepted formats.

### CLI and REST

Binaries parse transport input, call app services, and render stable output. They do not decode
domain resource variants, load catalog rows directly, or build operation configs.

## Persistence contract

Replace the current development migrations with one clean baseline migration. No deployed schema
compatibility is required.

```sql
CREATE TABLE catalog_values (
    name           TEXT  NOT NULL,
    schema_id      TEXT  NOT NULL,
    digest         TEXT  NOT NULL,
    canonical_json BYTEA NOT NULL,
    PRIMARY KEY (name, schema_id, digest)
);
```

Add checks for the catalog-name grammar, valid bounded schema/digest text, and canonical payload
length `1..=262144`. Use `BYTEA`, not `JSONB`, because exact canonical bytes are authoritative.

Apply the existing append-only trigger contract to `catalog_values` so `UPDATE`, `DELETE`, and
`TRUNCATE` fail. The storage implementation must provide only these raw operations:

1. Append a heterogeneous raw-row batch in one `READ COMMITTED` transaction:
   - receive rows already prepared by app;
   - reject invalid names, identities, bounds, non-canonical JSON, and digest mismatches before or
     during the transaction without decoding a domain type;
   - `INSERT ... ON CONFLICT DO NOTHING`;
   - read every exact key back and compare bytes;
   - commit only if every row matches exactly.
2. Load one exact raw row:
   - query by name, caller-supplied schema id, and digest;
   - enforce the payload bound;
   - parse canonical JSON and recompute the digest;
   - return verified canonical bytes without importing or decoding `T`.
3. List metadata only in `(name, schema_id, digest)` order with keyset pagination and a maximum page
   size of 100.
4. Export one exact row only after canonical/digest verification.

App setup performs typed publication preparation before operation 1: strict decode of the closed
setup variant, `ValidatedConfig::new`, shared canonical serialization, the 256 KiB bound, schema-id
and digest derivation, and the defense-in-depth secret-field scan. It returns the typed
`CatalogRef<T>` to the caller and passes a raw row to storage.

App launch preparation performs typed resolution after operation 2: derive `T::schema_id()` from
the request type, strict-decode the verified bytes, run `T::validate()`, reserialize through the
shared canonical path, and require byte equality. Wrong-type and semantic-validation failures are
therefore app resolver errors, not storage errors.

There is no insert/update timestamp, numeric revision, current flag, scope column, mutable alias,
name-only read, latest query, soft delete, or garbage collection.

The app-owned secret scan is not a second semantic validator. It is a publication-boundary guard
against fields whose normalized names indicate passwords, private keys, mnemonics, secret material,
unlock material, or credentials, and against URI-like string values containing user information. It
must return only the rejected field path, not its value. Registered setup types and negative tests
remain the primary defense.

## Setup contract

The first setup surface is CLI-only:

```text
mfm setup import --file organization.toml
mfm setup list [keyset options] [--limit N]
mfm setup export --name N --schema-id S --digest D --output PATH
```

There is no format flag. Import parses TOML only. Export requires a new output path, writes the exact
canonical JSON bytes, and fails if the path already exists. List returns identities only, never
payloads.

The strict import shape is:

```toml
[[values]]
name = "portfolios/treasury"
kind = "portfolio"

[values.value]
# fields of the selected concrete type
```

Use one private, deny-unknown-fields, tagged enum in app setup code. The initial variants are the
smallest set needed by all current entry points and the representative composed workflow:

- `portfolio` -> `PortfolioConfig` after semantic normalization;
- `evm_contract_context` -> `EvmContractContext`;
- `evm_deploy_action` -> `DeployAction`;
- `evm_configure_action` -> `ConfigureAction`;
- `evm_validate_action` -> `ValidateAction`;
- `evm_import_deployed` -> `ImportDeployedSpec`;
- `evm_import_configured` -> `ImportConfiguredSpec`.

These existing complete configs and contract pieces are the v1 catalog value model. Do not add
network, account-set, address-set, token-set, or signer-intent wrapper types in this implementation.
The composed operation derives collectors from one portfolio; finer decomposition is a future RFC
only after a concrete second consumer exists.

Add `MfmConfig` to `EvmContractContext` using its existing descriptor and strict deserializer. Do not
create wrapper catalog-value types for any item above.

Audit every registered type's full nested serde tree and make unknown-field rejection intrinsic to
those domain types with `deny_unknown_fields` or an existing strict custom deserializer. Do not
recreate authored-config's parse/round-trip comparison as a second strictness path in setup code.

Validate and canonicalize the full document before starting the batch transaction. Duplicate exact
items are idempotent. Duplicate names with different values append distinct digests. Duplicate items
inside one import that disagree on the same exact identity fail before SQL.

REST catalog mutation and an onboarding UI are out of scope until authentication and authorization
are designed. Do not add unauthenticated setup endpoints as a shortcut.

## Exact entry-point contract

The target inventory is:

| Exact id | Request inputs | Complete operation config construction |
|---|---|---|
| `mfm.evm.contract/deploy@1` | context ref + deploy-action ref | Pure `build_deploy_config` |
| `mfm.evm.contract/configure@1` | context ref + deployed-import ref + configure-action ref | Pure `build_configure_config` |
| `mfm.evm.contract/validate@1` | context ref + configured-import ref + validate-action ref | Pure `build_validate_config` |
| `mfm.evm.contract/lifecycle@1` | context ref + deploy/configure/validate action refs | Pure `build_lifecycle_config` |

Every request type is strict, typed, and has a stable request schema id. The four contract
entry-config types move to `mfm-op-evm-contract-lifecycle` and gain ordinary constructors from their
existing pieces so builders do not deserialize through JSON or write private fields indirectly.

The state-owned contract action, import, signer-intent, transaction-policy, and receipt-policy types
move to `mfm-state-evm-contracts`. Keep `EvmContractContext`, assertions, scalar types, lifecycle
values, provenance, and evidence in `mfm-evm-contract-model`. Preserve explicit schema identities,
move tests to the new owners, and delete `mfm-evm-contract-config` atomically with no re-exports.

The ownership move is exact:

| Target owner | Types |
|---|---|
| `mfm-state-evm-contracts` | `EvmSignerIntent`, `EvmTransactionStyle`, `EvmTransactionPolicy`, `ReceiptRetryPolicy`, `DeployAction`, `ConfigureAction`, `ValidateAction`, `ImportDeployedSpec`, `ImportConfiguredSpec` |
| `mfm-op-evm-contract-lifecycle` | `EvmContractDeployEntryConfig`, `EvmContractConfigureEntryConfig`, `EvmContractValidateEntryConfig`, `EvmContractLifecycleEntryConfig` |

Move the receipt-limit constants with `ReceiptRetryPolicy` and make them private unless a concrete
public consumer requires them. The state crate gains direct `alloy-primitives` and `mfm-evm-core`
dependencies; the operation crate gains direct `serde`. Adapter consumers switch to the state
owner, and the unused integration-test dependency on the deleted crate is removed. No model-to-state,
state-to-operation, operation-to-adapter, or storage-to-domain edge is introduced.

Collector configs remain usable directly by their operation crates. The first catalog design does
not invent separate network/account/address-set wrapper types merely to split those already-complete
configs. The composed portfolio workflow derives collector configs from one `PortfolioConfig`,
which removes the repeated network and wallet data in the motivating scenario. More granular
catalog types require a demonstrated second consumer.

The superseded standalone collector defaults must not be restored. Internal collection policy is
closed in the certified operation graph rather than supplied by public collector requests.

## Collect-then-report composition

Create `crates/ops/portfolio-collect-report-op`. It owns only the higher-level deterministic
operation and its request/config assembly. It depends on the BTC collector, EVM collector,
portfolio tracker, and their shared domain/state types; none of those lower operations depend back
on it.

The pure builder must:

1. Normalize and validate the loaded `PortfolioConfig`.
2. Derive one BTC collector config for each configured Bitcoin network with relevant wallet
   subjects.
3. Derive one EVM collector config for each configured EVM network with relevant account subjects
   and native-asset decimals.
4. Apply only the explicit read/coverage policy from the request.
5. Reject missing network identity, duplicate subjects, unsupported wallet/symbol joins, empty
   collection sets, and any mismatch before graph expansion.
6. Produce one complete `CollectThenReportConfig` containing the portfolio and child operation
   configs.

The operation graph must call the registered typed collector operations, wait for their typed batch
summaries, and then call the report graph. Do not express this dependency in catalog metadata or app
launch code.

Use one shared portfolio-report expansion helper. Introduce the minimum typed state/value support
needed to make report readiness explicit:

- the standalone report path creates a typed `PortfolioInputsReady` value with a small pure state;
- the composed path creates the same value with a pure state that consumes every collector summary;
- the first report state consumes `PortfolioInputsReady`;
- the remaining report topology is shared.

This produces real typed graph edges and ensures collector fact commits precede fact-backed report
selection. Do not duplicate the report graph, poll runs, launch child runs from a state, or add a
catalog "after_success" field.

Retain the independent `portfolio_snapshot`, BTC collector, and EVM collector entry points.

## Launch evidence contract

Replace the current evidence shape with:

```text
EntryPointLaunchEvidence
  entry_point_id
  catalog_sources[]

CatalogSourceEvidence
  name
  schema_id
  digest
```

Validate and sort sources by `(name, schema_id, digest)`, then deduplicate exact repeats. These fields
are `RunAdmitted` audit evidence only. They must not enter `RunIdentityMaterialV1`; run identity
remains exactly certified spec hash, store scope, and invocation-key digest. Adding or removing an
unrelated compiled entry point must not alter another entry point's admission evidence.

Overwrite the current event model, schema descriptors, codecs, runtime launch-evidence JSON,
snapshots, and fixtures. There is no old-field decoder and no second event version for this cutover.

Internal test-support launches use an exact internal test entry-point id and an empty source list.

## Crate and dependency budget

The reviewed baseline at `bacbb530` is:

| Metric | Baseline |
|---|---:|
| Workspace packages | 53 |
| Unique direct workspace dependency pairs | 411 |
| Tracked Rust lines | 214,392 |
| Top-level public declarations under `crates` and `bin` | 1,754 |

Planned package changes are fixed:

| Add | Delete |
|---|---|
| `mfm-catalog-model` | `mfm-authored-config` |
| `mfm-op-portfolio-collect-report` | `mfm-portfolio-config` |
|  | `mfm-evm-contract-config` |

The expected final package count is therefore 52. Recompute every metric from the implementation
baseline before commit 1 and after commit 4; do not rely on the reviewed numbers if intervening work
changes the tree. Report every direct dependency-edge increase and the reason it is necessary.

Apply this ownership budget throughout implementation:

- no new crate per catalog resource family;
- concrete resource and complete-config types remain with their model, state, or operation owner;
- `mfm-runtime-config` remains the only deliberately config-named package because it enforces the
  runtime secret/indirection boundary;
- the closed setup-import enum remains private to app input assembly;
- any proposed config crate must identify a stable abstraction that cannot live with its semantic
  owner;
- do not add wrappers, facades, or re-export crates to make moved types appear backward-compatible.

## Progressive commit plan

### Preparation before commit 1

Record the baseline without editing generated reports into the repository:

```text
git status --short
git rev-parse HEAD
git ls-files '*.rs' | xargs wc -l
rg -n '^pub (struct|enum|trait|type|fn|const) ' crates bin | wc -l
cargo metadata --no-deps --format-version 1
```

For the metadata baseline, record both package count and unique direct workspace dependency pairs.

The working tree must be clean except for the implementation work. Do not absorb unrelated user
changes.

### Commit 1: `add raw postgres config catalog`

Owned surfaces:

- `crates/kernel/values`;
- `crates/kernel/program` canonical binding call site;
- new `crates/catalog-model`;
- renamed `crates/storages/postgres`;
- workspace manifests, SQLx metadata, architecture metadata tests, storage README references,
  `docs/architecture.md`, and the PostgreSQL storage section of `docs/design.md`.

Work:

1. Add shared `ValidatedConfig::canonical_json` and move program config binding onto it.
2. Add `CatalogName` and `CatalogRef<T>` with strict construction and serde tests in
   `mfm-catalog-model`.
3. Rename the PostgreSQL crate and concrete store names everywhere, with no aliases.
4. Squash the development migrations into one baseline containing existing schema plus
   `catalog_values` and append-only protection.
5. Implement raw transactional batch append, exact raw-row load, bounded metadata list, and exact
   raw export. Use only minimal storage-owned key/row types and kernel identities/canonical bytes.
6. Update schema validation and SQLx offline metadata.
7. Add focused PostgreSQL parity tests.
8. Extend metadata contracts to reject `mfm-storage-postgres -> mfm-catalog-model` and any storage
   dependency on domain-model/domain-config crates.

Required tests:

- valid and invalid catalog names;
- reference serde round trip and unknown-field rejection;
- shared canonical serialization produces stable bytes for program binding and later app setup use;
- same exact insert is idempotent;
- same name/schema with changed content creates another digest;
- conflicting exact key with different bytes fails closed;
- invalid name/identity/bounds/canonical JSON/digest or SQL failure leaves no partial rows;
- update, delete, and truncate are rejected;
- wrong exact schema key is absent; wrong digest, malformed or non-canonical bytes, and oversized
  bytes are rejected on raw load;
- list ordering, keyset pagination, and page-size cap;
- exact export returns stored verified bytes;
- errors never include canonical payloads.

Commit only after all three mandatory gates pass.

### Commit 2: `add catalog resolution and setup`

Owned surfaces:

- `crates/app/src/config_setup.rs` and narrow app exports;
- app-owned typed catalog resolver;
- `bin/cli/src/commands/setup` and CLI output/error contracts;
- `crates/evm-contract-model`, `crates/states/evm-contracts`, and
  `crates/ops/evm-contract-lifecycle-op`;
- current public operation crates;
- portfolio state/model/tracker config simplification;
- workspace manifests, lockfile, metadata contracts, and dependency residue checks;
- setup examples and directly relevant docs.

Work:

1. Add the app-owned generic resolver over the concrete raw `PostgresStore` API. It derives the
   expected schema id, strict-decodes, validates, recanonicalizes, and returns ordinary `T`.
2. Add the strict private setup document enum and all-or-nothing import service. App performs typed
   validation, canonicalization, bounds, digest derivation, and secret scanning before storage.
3. Add CLI import/list/export without domain logic in the binary.
4. Add `MfmConfig` to `EvmContractContext`.
5. Move the nine state config/support types from `mfm-evm-contract-config` to
   `mfm-state-evm-contracts`; move the four entry configs to
   `mfm-op-evm-contract-lifecycle`; add ordinary entry-config constructors; split tests by owner;
   delete the old crate, dependency edges, and manifest entry in the same commit.
6. Make every registered setup value and nested type reject unknown fields at its own serde boundary.
7. Add strict operation-owned request types and pure contract config builders.
8. Change `PortfolioTrackerWorkflowOperation::Config` to `PortfolioConfig` directly; delete
   `PortfolioWorkflowConfig` and its conversions.
9. Add the request types and planning functions needed by the future private app dispatch, but do
   not expose a second app, CLI, or REST launch path. Commit 4 switches public ingress once.
10. Remove collector `Default` implementations and replace test use with explicit fixtures.
11. Add one complete setup-import TOML fixture covering the initial resource kinds.

Required tests:

- setup unknown kind/field at every registered nested family, float, oversized file/value, duplicate
  conflict, and secret-like field rejection;
- missing row, wrong schema type, semantic validation failure, and any recanonicalized byte mismatch
  fail closed in the app resolver;
- normalization gives stable portfolio digest for semantically reordered input;
- dry preparation of the entire import occurs before any SQL write;
- import output reports exact name/schema/digest only;
- export refuses an existing output path and writes byte-identical canonical JSON;
- every request rejects missing digest and unknown fields;
- contract builders preserve all fields and reject incompatible context/import/action joins;
- EVM action/import strict-serde and validation tests pass under the state owner; entry-config and
  builder tests pass under the operation owner; no `mfm-evm-contract-config` reference remains;
- operation crates retain no app, SQLx, runtime-config, or PostgreSQL dependency;
- portfolio tracker certified topology remains report-only with direct `PortfolioConfig`.

Commit only after all three mandatory gates pass.

### Commit 3: `add typed collect then report operation`

Owned surfaces:

- new `crates/ops/portfolio-collect-report-op`;
- minimal readiness value/state support in `crates/states/portfolio`;
- shared expansion support in `crates/ops/portfolio-tracker-op`;
- operation registration metadata needed by typed program expansion;
- focused operation and PostgreSQL parity tests;
- `docs/portfolio-collect-then-report.md` target workflow description.

Work:

1. Implement the explicit composed request and complete config builder.
2. Implement typed collector-summary-to-report-readiness dependencies.
3. Share the report graph expansion between standalone and composed operations.
4. Register and certify the parent operation and its child operation dependencies.
5. Add a one-parent-operation parity scenario. Delete the old public-ingress multi-start scenario in
   commit 4 while retaining independent collector/report behavior tests.

Required tests:

- pure derivation for EVM-only, BTC-only, and dual-network portfolios;
- mismatch, duplicate, missing native symbol/decimals, and empty collection failures;
- graph lineage contains parent, collector children, and report child;
- every report path is downstream of all collector summaries;
- one PostgreSQL run, launched through the existing direct certified-program test helper, records
  collector facts and produces the final report without requiring the commit-4 entry-point
  dispatcher;
- no external shell/app orchestration is used by the composed test;
- standalone report remains report-only and collector operations remain independently plannable;
- the state crate has no catalog/runtime-config dependency; the operation crate may depend on
  `mfm-catalog-model` only for pre-planning request types, while its `Operation::Config`, expansion,
  child configs, and certified graph contain no `CatalogRef<T>`.

Commit only after all three mandatory gates pass.

### Commit 4: `replace direct entry point config ingress`

This is the atomic breaking cutover. It may be large because the old and new public contracts must
not coexist in a committed tree.

Owned surfaces:

- `crates/app/src/entry_point.rs`, `entry_points.rs`, `lib.rs`, services, and tests;
- `crates/kernel/events`, `crates/kernel/store`, and runtime launch-evidence rendering;
- CLI run start and operation discovery;
- REST run start;
- all operation descriptor constants and authored planners;
- integration tests and fixtures;
- workspace manifests and lockfile;
- README, app/CLI/REST docs, `docs/architecture.md`, `docs/design.md`, examples, and
  `.gitignore`.

Work:

1. Replace public registry machinery with the private exact `EntryPoint` enum and exhaustive
   preparation.
2. Make launch preparation async over the concrete `PostgresStore`; decode the selected request,
   resolve exact refs, call only necessary builders, certify, and return the existing run launch
   request directly.
3. Delete app-level duplicate launch evidence and wrapper types where `RunLaunchRequest` already
   carries the required material.
4. Replace kernel launch evidence and all current codecs/fixtures with exact entry point plus sorted
   sources.
5. Replace CLI run-start flags with required `--entry-point` and `--request` JSON file. Keep
   `--runtime-config` separate.
6. Replace REST body with `entry_point`, strict JSON `request`, and optional `invocation_key`.
7. Change `ops list` to exact ids plus request schema ids.
8. Delete `crates/authored-config`, `crates/portfolio-config`, their tests, and every dependency.
9. Delete operation descriptor constants, latest/version resolution, accepted formats, source
   format digests, old error mappings, old config fixtures, and direct authored planners.
10. Delete old per-operation TOML examples and replace request examples with exact refs. Keep the
    runtime config example.
11. Remove the root `/*.toml` ignore convention and retain only explicit local setup/runtime ignores.
12. Rewrite behaviorally valuable tests; delete tests that assert only removed compatibility.
13. Regenerate `Cargo.lock` and SQLx metadata as required.

Required tests:

- all eight exact entry points prepare and launch from catalog-backed requests;
- unknown and unversioned entry-point ids fail;
- no latest selection exists;
- CLI and REST strict JSON requests resolve to identical complete config/spec material;
- old CLI flags are rejected by clap;
- old REST fields and TOML string config are rejected as unknown fields;
- missing row, wrong digest, wrong type, catalog corruption, and relational builder failures map to
  stable redacted CLI/REST errors;
- launch evidence sources are sorted/deduplicated and round-trip through current event codecs;
- adding an unrelated entry-point summary does not alter another launch's admission evidence;
- `RunIdentityMaterialV1` remains limited to certified spec hash, store scope, and invocation-key
  digest;
- resume, replay, status, stream inspection, and public-output rendering do not query catalog
  values;
- after admission, revoking catalog-table `SELECT` while retaining run-table access does not block
  resume/replay of that run;
- old event evidence fields are rejected, not decoded;
- all retained end-to-end behavior tests use catalog seeding and exact requests.

Commit only after all three mandatory gates pass.

## Mandatory deletion checklist across commits 2 and 4

Delete these directories:

```text
crates/evm-contract-config       # commit 2
crates/authored-config
crates/portfolio-config
```

Delete these current abstractions and all references:

```text
AuthoredConfig
AuthoredConfigFormat
NormalizedAuthoredConfig
EntryPointDescriptor
PublicOpName
OpVersion
EntryPointOpId (app-level structured id)
LaunchableOp
EntryPointPlannerAdapter
EntryPointOpRegistry
EntryPointOpError
EntryPointRunLaunchInput
PreparedEntryPointRunLaunch
EntryPointLaunchEvidence (app-level duplicate)
production_entry_point_op_registry
register / register_arc entry-point helpers
resolve_latest
registry_digest
entry_point_registry_digest
accepted_config_formats
TOML_JSON_AUTHORED_CONFIG_FORMATS
PortfolioSnapshotAuthoredConfig
PortfolioSnapshotCanonicalConfig
PortfolioSnapshotBuildReport
PortfolioWorkflowConfig
ProductionRunStore
ProductionPostgresSchema
connect_production_run_store
```

Delete all public operation descriptor constants, including:

```text
BTC_ADDRESS_BALANCE_ENTRY_POINT
EVM_NATIVE_BALANCE_ENTRY_POINT
PORTFOLIO_SNAPSHOT_ENTRY_POINT
CONTRACT_DEPLOY_ENTRY_POINT
CONTRACT_CONFIGURE_ENTRY_POINT
CONTRACT_VALIDATE_ENTRY_POINT
CONTRACT_LIFECYCLE_ENTRY_POINT
CONTRACT_ENTRY_POINTS
```

Delete the old CLI/REST fields and helpers:

```text
--op
--op-version
--config
--config-format
ConfigFormatArg
RestConfigFormat
op_version
config_format
```

Delete tests and fixtures whose only purpose is:

- TOML/JSON parity for per-operation launch config;
- path/format sniffing or fallback;
- accepted-format reporting;
- latest operation resolution;
- old config envelopes or deleted fields;
- old event evidence decoding;
- fake `LaunchableOp` registry behavior.

Retain and rewrite tests for real planning, certification, event persistence, runtime behavior,
fact selection, contract lifecycle safety, CLI/REST stable errors, and public outputs.

Run these residue searches before the commit and require no unexplained hits outside historical RFC
current-state text and this plan:

```text
rg -n 'mfm[-_]authored[-_]config|mfm[-_]portfolio[-_]config|mfm[-_]evm[-_]contract[-_]config' .
rg -n 'AuthoredConfig|NormalizedAuthoredConfig|EntryPointDescriptor' crates bin tests
rg -n 'resolve_latest|registry_digest|accepted_config_formats' crates bin tests
rg -n 'op_version|config_format|--op-version|--config-format' bin tests docs
rg -n 'PostgresRunStore|ProductionRunStore|ProductionPostgresSchema|mfm_stream_store_postgres' .
cargo metadata --no-deps --format-version 1
```

Historical explanation in `RFC_CONFIG.md` may retain old names only where clearly labelled as the
pre-cutover state.

## Verification matrix

### Pure/unit verification

- `mfm-values`: one canonical config implementation and stable digest behavior.
- `mfm-catalog-model`: exact typed name/reference validation and serde.
- app: typed publication preparation and exact typed resolution over verified raw rows.
- PostgreSQL storage: raw row identity, canonical/digest integrity, append atomicity, list, and
  export without domain decoding.
- domain/ops: strict request decoding, relation validation, deterministic builders.
- portfolio state/op: typed readiness and shared report expansion.
- kernel events/store: exact evidence schema and current-only codecs.

### Focused Cargo verification while developing

Run the smallest relevant commands after each edit group, for example:

```text
cargo test -p mfm-values
cargo test -p mfm-catalog-model
cargo test -p mfm-storage-postgres
cargo test -p mfm-app
cargo test -p mfm-op-portfolio-collect-report
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo fmt --all -- --check
cargo check --workspace
```

Use explicit `DATABASE_URL` for focused PostgreSQL tests. Do not wait for a full Nix gate to discover
basic compile errors.

### PostgreSQL verification

- schema drift and SQLx offline metadata;
- catalog append-only enforcement;
- transaction atomicity and idempotence;
- exact load/export corruption detection;
- run admission and resume independence after catalog resolution;
- one-run collect-then-report parity.

### Boundary verification

Extend cargo metadata/architecture tests to assert:

- `mfm-catalog-model` is a `domain-model` crate and depends only on kernel crates;
- `mfm-storage-postgres` does not depend on `mfm-catalog-model`, domain-model, or domain-config
  crates and does not import `CatalogRef<T>`, `MfmConfig`, or `ValidatedConfig<T>`;
- operation/state crates have no app, PostgreSQL, SQLx, CLI, REST, or runtime-config dependency;
- binaries do not depend directly on domain operation config crates for setup decoding/building;
- only app calls catalog persistence operations on the concrete PostgreSQL store;
- kernel, runtime, replay, states, certified operation/state configs, and event payloads do not
  depend on `mfm-catalog-model` and do not contain `CatalogRef<T>`;
- only app setup/resolution and operation-owned pre-planning request types may depend on
  `mfm-catalog-model`; storage must not;
- no crate named `mfm-authored-config`, `mfm-portfolio-config`, or `mfm-evm-contract-config`
  remains, and `mfm-runtime-config` retains category `runtime-config`.

### Security verification

- reject passwords, mnemonics, private keys, unlock paths, credential-bearing URLs, and unknown
  secret-like fields in setup imports;
- verify signer refs and public signer addresses remain allowed while secret providers remain
  runtime-only;
- assert errors contain identity/path information but no rejected values or canonical payload;
- assert catalog bytes never appear in logs, event diagnostics, CLI errors, or REST errors;
- fuzz or property-test canonical round trips and name parsing if an existing repository harness is
  available; do not add a new fuzz framework solely for this change.

## Final merge-readiness pass

After commit 4:

1. Confirm `git status --short` is clean.
2. Run all residue searches in the deletion checklist.
3. Recount packages, unique direct workspace dependency pairs, Rust LOC, and public declarations
   with the same baseline commands.
4. Inspect `cargo metadata` to confirm the three removed config crates and old PostgreSQL package
   are absent and the final package count is 52.
5. Run:

   ```text
   nix run .#ci
   ```

6. Report:
   - the four implementation commits;
   - all gate results;
   - baseline and final package/dependency-pair/Rust-LOC/public-declaration counts;
   - removed crates/public types/code paths;
   - any deliberate variance from this plan, with the approving architecture decision.

Do not call the work complete if final CI is red, old paths remain, or the implementation requires a
fallback to launch an operation.

## Agent coordination guidance

Storage, operation, and integration agents may work in parallel only on disjoint files. Recommended
ownership is:

- storage agent: commit 1 only;
- setup/domain agent: app resolver, setup importer, and app-owned publication checks in commit 2;
- EVM ownership agent: atomic state/operation type move and old crate deletion in commit 2;
- operation agent: operation requests/builders and portfolio simplification in commit 2;
- composition agent: commit 3;
- integration owner: all of commit 4 because app, event evidence, CLI, REST, deletion, and docs form
  one atomic contract cutover.

Every worker must be told that other agents share the tree, must not revert unrelated edits, and
must not create compatibility shims to reduce merge conflicts. The integration owner reviews every
new public type and rejects any type that merely wraps another type without enforcing a distinct
invariant.
