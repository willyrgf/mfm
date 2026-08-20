# Design: symmetric CLI and REST client surfaces

Status: reviewed target design for a later implementation wave. REST implementation remains blocked
on explicit approval of the same-user socket boundary below. This wave changes this document only.

The implemented EVM collector e2e established one real user path, but it also exposed the product
boundary problem: the CLI currently owns deployment parsing, environment resolution, concrete
adapter composition, entry-point planning, and rendering, while REST is a stub. The target is one
typed, transport-neutral Application surface, with CLI and REST as two thin renderings of the same
user use cases.

The foundational target was reviewed by four independent architects (architecture ownership,
API/product, security/custody, and implementation feasibility) on 2026-08-20. The coordinating
agent checked their findings against the current tree and resolved their disagreements below.

## User inputs and architecture rulings that bind this design

1. Configs form a **named durable catalog**. A run selects a stored config; it does not carry an
   inline config document.
2. REST is a **local single-user daemon with no authentication and no authorization**. That claim
   requires a mechanical same-user boundary. The earlier documented-only TCP-bind decision cannot
   receive architecture/security approval; the target below uses an owner-only Unix socket and
   remains pending explicit user confirmation.
3. This wave produces a design document only. No implementation follows in this wave.
4. Deployment is a strict TOML operator bootstrap found through the conventional XDG/HOME path or
   an argv override. It contains environment resolver names only and has no product lifecycle.
5. Config import selects the entry point through the document's required tag. Run start selects a
   stored config revision, never an entry point. CLI may generate a RunId before the Application
   call; REST, Application, and Runtime continue to require an explicit caller-provided RunId.

---

# Situation and verified facts

1. `IMPL_PLAN_EVM_COLLECTOR_E2E_TEST.md` is implemented on branch `evm-coll-e2e`. The CLI now has
   `init`, `snapshot`, and `show`, one combined JSON file, a production JSON-RPC EVM provider, shared
   Portfolio assembly, PostgreSQL provisioning, and a managed live e2e.
2. `bin/cli/src/main.rs` still owns config-file parsing, `std::env::var`, PostgreSQL/provider
   construction, the one-route assembly, Application error translation, and rendering. This is the
   duplication REST would otherwise repeat.
3. REST is a five-line unavailable diagnostic. It has no compatibility burden and no listener.
4. `Application` exposes Portfolio-specific `start_portfolio`, generic Runtime `resume`, and `read`.
   No production caller uses `resume`.
5. `Runtime` owns the sole semantic fold and holds only `Arc<dyn Store>`. `Store` owns exactly
   `load_run` and `append_run`; it has no enumeration contract.
6. `mfm_run_heads` already contains `run_id`, `head_sequence`, and `total_bytes`; its foreign key
   identifies the head frame and therefore its `head_digest` without parsing a frame.
7. Every run-history PostgreSQL gate is scoped to schema `public`. An independently gated
   `mfm_catalog` schema is invisible to the run-history contract.
8. `plan_snapshot` already accepts a non-empty target slice that is strictly sorted and unique by
   chain id. `register_evm_reads` can register the three capability bindings once per distinct
   `EvmPhysicalTarget` before the Runtime assembly freezes.
9. `PlainCanonicalJsonBytes::content_digest()` produces
   `content:sha256-jcs-v1:<64 hex>`. The previous draft's `sha256-v1` catalog constraint was wrong;
   `sha256-v1` is for exact retained object/frame bytes, not a canonical JSON document identity.
10. `Keystore` is in-memory only and deliberately `!Send + !Sync`. Existing trybuild tests already
    prove that thread-affinity property. There is no sealed at-rest format, passphrase lifecycle, or
    signing implementation.
11. `reqwest` currently follows redirects, permits implicit system-proxy resolution unless
    explicitly disabled, and has no TLS feature. `sqlx` is built with `tls-none`. A shared
    deployment composer must not claim exact endpoint authority while those paths remain implicit.
12. EVM domain chain ids are `u64`, but the implemented production path and fixtures use only small
    positive values; no current requirement exercises the range above TOML's positive `i64` limit.

## Architect review record

| Question | Resolved design | Reason |
| --- | --- | --- |
| Shared contract owner | `mfm-app` | It already owns trusted assembly and run use cases; a transport-surface DTO crate would add no boundary. |
| Catalog/index port owner | new `mfm-catalog` crate | Named mutable custody and enumeration have their own errors, atomicity, capacity, and persistence lifecycle. They do not belong to append-only `Store`. |
| `Store` changes | none | Runtime must continue to receive only complete-prefix load and exact-head append authority. |
| Stored config shape | complete, versioned config document | One catalog works for every entry point without generic merging or per-entry-point storage code. |
| Config selection at execution | exhaustive `Current` or `Exact` selection | Interactive CLI use may atomically select the name's current revision; retrying/scheduled callers can pin the exact JCS digest. |
| Config deletion | atomic compare-and-delete by name and digest | An unconditional delete can delete a newer re-imported revision observed by a stale caller. |
| Multi-route representation | public binding mappings in Deployment; a route array in the config document | Runtime assembly must know `(chain_id, endpoint_id)` before freeze, and Portfolio planning may use several chains. |
| Deployment role | strict operator-only `deployment.toml` bootstrap | It composes private authority but has no product identity, API, or lifecycle. |
| Public binding discovery | public tagged bindings, never Deployment/env metadata | Clients need selectable capability identities, not environment resolver names, URLs, or socket configuration. |
| Run listing | cheap mechanical heads only | Status, entry point, recency, and config provenance require semantic folds or a separate derived projection. |
| Run JSON | preserve the `RunViewState` sum | A status string plus correlated optional fields violates the code-quality policy and loses terminal contract/value refs. |
| Keystore | separate custody RFC; CLI administration only | The current in-memory owner cannot persist across CLI invocations, and unauthenticated HTTP must not carry custody mutation. |
| PostgreSQL authority | separate provisioning and runtime credentials | A long-lived daemon needs exact DML authority, not table-owner/DDL authority. |
| REST locality | owner-only Unix socket; pending user confirmation | Loopback TCP without authentication is not a same-user boundary; documentation does not enforce caller identity. |

---

# Target design

## 1. Product nouns and the symmetry rule

There are three user-facing nouns:

- **Entry point** — a supported, versioned execution contract such as
  `mfm.portfolio/snapshot@1`. The repository's source-authoring `Operation` remains an internal type
  erased during Program expansion; it is not a transport resource.
- **Config** — a named catalog entry containing one complete canonical config document.
- **Run** — one durable execution identified by one explicit `RunId` at the Application/Runtime
  boundary. A client transport may create that identity before making the call.

CLI and REST are symmetric at the Application use-case boundary, not byte-identical protocols:

> Every shared user use case is one typed `Application` function. Each binary bounds and parses its
> transport grammar into the same checked request types, calls exactly one Application function, and
> exhaustively renders its result or shared `RequestError`. A transport-only concern remains local
> to that binary and is named as an asymmetry.

This rejects two tempting but incorrect interpretations:

- The CLI does not call REST. It remains usable as a direct local client and shares the Application
  contract with the daemon.
- Application does not accept arbitrary transport strings. Binaries own argv/path/header/query
  parsing; checked shared constructors own the actual value/document grammar.

## 2. Deployment bootstrap versus stored config material

The split is semantic, not based on whether a string happens to resemble a URL:

- **Deployment bootstrap material** contains only environment resolver names for the runtime store
  and public-binding-to-private-adapter mappings. Composition resolves those names into database/RPC
  locators and provider handles. It is local, operator-authored, never stored in the catalog, never
  returned by REST, and never admitted into Program/C0/history.
- **Config material** is deterministic, bounded, secret-free domain input plus stable public
  references that select pre-bound capabilities. It is safe to canonicalize, digest, persist, list,
  and admit.

A stored field is safe because no code interprets it as an environment name, URL, filesystem path,
host, port, or socket address. Field-name rejection cannot prove that arbitrary public text contains
no secret; the guarantee is the absence of any ambient resolver behavior on the stored path.

### Deployment bootstrap

Deployment is one narrow, operator-only bootstrap input to Application composition. It is not a
product resource: it has no public identity or digest, is never catalogued, and has no import, list,
show, update, delete, or REST lifecycle. The conventional filename is `deployment.toml`.

Both binaries pass an optional argv override to the one app-owned path resolver:

1. `--deployment <PATH>` uses exactly that caller-supplied path and does not inspect XDG or HOME;
2. otherwise, an absolute, non-empty `$XDG_CONFIG_HOME` resolves to
   `$XDG_CONFIG_HOME/mfm/deployment.toml`;
3. if XDG is unset, empty, or relative, an absolute, non-empty `$HOME` resolves to
   `$HOME/.config/mfm/deployment.toml`; and
4. if neither default base is usable, composition fails with one redacted local bootstrap error.

Fallback is based only on whether the XDG base value is usable, not on file existence. Once a path
is selected, a missing or unreadable file fails; the resolver never probes the HOME path afterward.

There is deliberately no `$MFM_DEPLOYMENT`, current-directory search, system-wide fallback, or
automatic file creation. `Deployment::load(override_path)` performs this resolution, reads at most
`MAX_DEPLOYMENT_DOCUMENT_BYTES + 1` asynchronously, then moves owned bytes into one immediately
awaited pure blocking parse/validation job. Path/environment/filesystem IO stays outside that job;
a join failure is one redacted `ComposeError`. The initial encoded maximum is 256 KiB.

The file is UTF-8 strict TOML 1.0 with one current schema. There is no format/version marker, include,
profile inheritance, merge, environment interpolation, or string substitution. TOML duplicate
keys/tables fail parsing; `deny_unknown_fields` applies at every typed table; and only the following
runtime store reference and public-binding-to-private-adapter mappings exist:

```toml
[store]
runtime_locator_env = "MFM_RUNTIME_STORE_LOCATOR"

[[evm_routes]]
chain_id = 1337
endpoint_id = "reth-dev"
adapter_locator_env = "MFM_EVM_ADAPTER_RETH_DEV"
```

A direct, pinned TOML parser dependency in `mfm-app` is justified by this operator boundary; the
implementation does not hand-roll TOML or enable editing/merge features.

`store` is required exactly once. `evm_routes` is the only optional collection and defaults to an
empty list when no array-of-table is present.

`evm_routes` contains at most 256 entries and is strictly sorted and unique by
`(chain_id, endpoint_id)`. More than one endpoint may be bound for one chain, but a single Portfolio
config selects at most one endpoint for each chain because `plan_snapshot` requires chain-unique
targets. The same order is the stable `BindingList` order and assembly-registration order. An empty
deployment route list is valid: it can serve catalog/discovery use cases, while run start fails
`BindingUnbound` before Store IO.

TOML integers are signed 64-bit values. The checked deployment chain-id type therefore accepts
exactly `1..=i64::MAX` and converts that positive value losslessly to the domain's `u64`; zero,
negative values, and out-of-range TOML integers are rejected. The EVM domain remains `u64` for
library use. No current production path or fixture requires its upper half, so the bootstrap does
not add a quoted-string alternative. If a concrete deployment needs it, the one current TOML schema
changes to a checked unsigned decimal string rather than supporting integer and string forms in
parallel.

This limit belongs only to the production Deployment parser. Domain values and JSON config
documents retain `u64`; an upper-half config remains valid for an injected library composer but is
necessarily `BindingUnbound` under the current TOML-backed production composer.

Environment names are 1–64 ASCII characters matching `[A-Z_][A-Z0-9_]*`. The TOML contains only
these resolver names, never a database URL, RPC URL, credential, TLS-root path, or other locator
value. Each value is resolved once into a bounded, adapter-specific, non-`Debug`, non-`Display`,
non-serializable private locator wrapper and consumed only by the PostgreSQL or provider
constructor. There is no recursive resolution: text such as `${OTHER_ENV}` inside TOML is not
expanded and is invalid as an environment name. Local errors may name the checked resolver name;
they never contain its value.

The REST socket, concurrency, and deadline remain REST-binary options rather than Deployment fields.
Application owns no socket.

The runtime store locator authenticates as the fixed PostgreSQL role `mfm_runtime`. `store init`
additionally receives one checked `--admin-store-locator-env <NAME>` argument and calls a separate
CLI-only provisioner. Before mutation, the provisioner parses both single-host locators, proves that
their secret-free server/database target is identical, and rejects ambiguous target forms. The
dedicated runtime role must already exist; the provisioner never creates or interpolates a
credential-bearing URL username.

The admin connection installs/owns the independently gated run-history and catalog objects and
grants `mfm_runtime` only `CONNECT`, schema `USAGE`, marker/table `SELECT`, frame/config `INSERT`,
head `INSERT/UPDATE`, and config `DELETE` where each port requires them. It revokes inherited public
authority needed to make that grant exact, including `TEMPORARY` on the dedicated database. Before
any DDL, the **admin** connection inspects the fixed target role and existing database posture; it
does not pretend its own `current_user` is the runtime role. Every subsequent **runtime** connection
gate additionally proves `current_user = 'mfm_runtime'`. Both checks require `NOINHERIT`, no role
memberships, no superuser/createdb/createrole/replication/bypass-RLS attributes, no
database/schema CREATE or database TEMPORARY authority, no database/schema/object ownership, and no
excess MFM-object privileges. The role name is a fixed SQL identifier, never derived from the URL.

Provisioning finishes by opening the runtime connection and proving both independent gates.
`Application::open` resolves the checked `runtime_locator_env` and each `adapter_locator_env`; the
admin locator has no listener-held code path and need not be present in the daemon environment.

### Exact endpoint authority

The composition cut closes every hidden network-authority path with one deliberately small backend
profile:

- EVM RPC always requires `https` with hostname/IP verification through Rustls against the route's
  private locator's checked TLS roots. Plaintext loopback is not an exception: a local process could
  impersonate the endpoint, forge observational evidence, or receive locator credentials. URL
  fragments, client certificates, and non-HTTP schemes are rejected.
- `JsonRpcEvmProvider` explicitly applies `.no_proxy()`, redirect `Policy::none()`,
  `.referer(false)`, `.retry(reqwest::retry::never())`, and `.https_only(true)`. With pinned reqwest
  0.12.28, both modes disable every reqwest aggregate/native/WebPKI root source and use
  `.use_preconfigured_tls(...)` to inject a version-matched Rustls `ClientConfig` built from the
  shared primitive's exact immutable `RootCertStore`. EVM-live owns protocol/client wiring, not root
  selection or loading.
  No environment proxy, redirect target, referer, implicit protocol-NACK retry, or second/additive
  root source can exercise different authority.
- PostgreSQL admits one single-host TCP URI and always requires `sslmode=verify-full`; even loopback
  plaintext is rejected because an impersonating local listener could request cleartext password
  authentication. Rustls verifies the exact hostname/IP against the private locator's checked TLS
  roots and retains the resulting immutable root store. Missing, opportunistic, verify-CA-only,
  multi-host, service/passfile, client-certificate, and unpinned custom-root profiles are rejected.
- URLs and TLS-root paths remain absent from every error, log, public view, and stored value; and
  provider construction has its own redacted `ComposeError` class.

Each resolved private locator is a bounded, versioned adapter-owned value containing its endpoint
URL and an exhaustive TLS-root choice: public WebPKI or an absolute PEM-file path plus required
`content:sha256-v1` exact-byte digest. That entire locator value comes from the named environment
variable; none of its fields appear in `deployment.toml`.

A small reusable live transport-security primitive owns the checked TLS-root spec and exact
immutable Rustls root-store loader, so PostgreSQL and EVM cannot implement different path, bound,
pin, or PEM rules. A PEM path is absolute UTF-8, 1–4096 encoded bytes, and contains no NUL. The
loader opens it as a no-follow regular file, reads at most 256 KiB, verifies the digest before
parsing, and returns either compiled WebPKI anchors or exclusively the PEM anchors, never their
union. The public certificate material is not secret, but the pin prevents a path replacement from
silently changing connection authority.

PostgreSQL connection authority belongs wholly to `mfm-storage-postgres`, not `mfm-app`.
`Application::open` resolves `runtime_locator_env` once and passes its opaque,
non-`Debug`/non-`Display` value to a checked storage constructor. The storage crate's private locator
parser never calls `PgConnectOptions::from_str`: SQLx seeds that path from `PG*`, may read `.pgpass`,
retains certificate/options inputs, and tracing-warns unknown query values. The parser exhaustively
requires scheme, username, an explicit non-empty password component, one host, database, and the
one `sslmode` parameter; duplicate or unknown components fail redacted. The adapter does not inspect
or reject `PG*`, home, passfile, or service state because none of it is an input.

SQLx's current `PgConnectOptions::new_without_pgpass()` is insufficient: it still reads `PG*`, can
probe default socket paths, and has no setters that clear every certificate/options field. Its stock
PEM option is also additive to compiled WebPKI roots, so it cannot implement an exclusive pinned-CA
variant. The implementation therefore pins a minimal reviewed SQLx patch that exposes both a
genuinely compiled-defaults-only `PgConnectOptions::new_without_env_or_files()` constructor and an
exclusive immutable Rustls `RootCertStore` input. The constructor performs no environment,
home-directory, passfile, service, DNS, socket-probe, or filesystem access. The exclusive-root path
replaces, rather than augments, compiled roots. The storage crate then sets every field from checked
input, including the explicit password, fixed application name, exact root store, and verify-full
SSL mode; SQL statement logging is disabled. Upstream adoption or removal of that patch requires the
same source audit, root-anchor assertions, and hostile-ambient regression suite, not a silent return
to a public ambient constructor or additive root loader.

The storage crate's direct `url` dependency and narrow SQLx patch are justified because together
they prove SQLx never sees an unreviewed URI or resolver input. The storage module owns target
normalization/equivalence, PostgreSQL TLS-mode wiring, the admin provisioner, and the runtime
role/ACL gates; it invokes the shared transport-security loader for the root source and never reads
that file itself. Tests mutate hostile `PG*`/home/passfile state concurrently and prove it is ignored
without access, authority change, or value leak.

Supporting cloud aliases, client certificates, or operating-system root discovery later adds a
separately checked private-locator variant; it does not widen Deployment or loosen this profile
implicitly.

## 3. Checked config documents

`mfm-app` owns an opaque `ConfigDocument`. Its public checked constructor is async: after one
constant-time encoded-length check, it moves owned bounded bytes into one immediately awaited pure
`spawn_blocking` job. The private synchronous parser is invoked only inside such `mfm-app` blocking
closures (including catalog revalidation). A blocking-task `JoinError` maps to the redacted
checked-input `internal`/500 row below; planner or revalidation join failure maps to
`RequestError::Internal`. The constructor performs, in order:

1. fixed encoded-size enforcement before allocation at each transport;
2. strict JSON parsing into a canonical representation (duplicate keys, floats, invalid number
   spellings, and excess depth rejected);
3. exhaustive typed deserialization into the private entry-point enum, including the domain and
   route-shape checks owned by those public input types; and
4. canonical-byte digesting as a checked `ConfigDigest` using `sha256-jcs-v1`.

`Application::import_config` then moves the owned document into one immediately awaited
`spawn_blocking` call and runs the full pure planner on its declared routes. That proves
selector/config agreement, route coverage, Program/C0 construction, and current planning limits;
the resulting Program/C0 is discarded. `PortfolioError::InvalidValue` is caller-invalid input, while
`InvalidContinuation` and `Program` are trusted planner failures and map to `Internal`/500. They are
never misreported as 422. Run start reloads the retained canonical document, revalidates its digest
as untrusted storage input, and plans it again in the same blocking shape; any now-invalid caller
precondition means `InvalidCatalog`, while the trusted planner variants retain the same
`Internal` mapping. This keeps the catalog opaque to Program semantics and ensures a corrupt row
cannot become trusted input.

```rust
#[derive(serde::Deserialize)]
#[serde(tag = "entry_point", content = "input", deny_unknown_fields)]
enum ConfigDocumentWire {
    #[serde(rename = "mfm.portfolio/snapshot@1")]
    PortfolioSnapshot {
        routes: EvmRouteSelection,
        selector: PortfolioSnapshotSelector,
        portfolio: PortfolioConfig,
    },
}
```

The required `entry_point` tag is the sole entry-point selector. Import validates it and derives
the `ConfigSummary.entry_point`; `Application::entry_points()` reports exactly the tags accepted by
the current document enum. Neither `ConfigSelection`, CLI `run start`, nor the REST start body has a
second entry-point field that could disagree with the retained document.

`EvmRouteSelection` contains 1–64 entries, is strictly sorted, and is unique by chain id. Each item
is `{chain_id, endpoint_id}` and derives the same `EvmPhysicalTarget` identity as the corresponding
Deployment binding, without resolving a URL. For Portfolio `@1`, its chains must exactly equal the distinct
chains demanded by the contained sources; neither a missing nor unused route is admitted. Import
never consults the current deployment; run start requires every selected target to be bound.

`@1` freezes the exact document-to-`(Program, C0)` derivation. Any change that alters representative
Program or C0 bytes requires a new entry-point id, even when the input JSON still deserializes.
Digest fixtures enforce that rule.

The initial application bound is `MAX_CONFIG_DOCUMENT_BYTES = 256 KiB`. A generated worst-case
accepted Portfolio document must prove the bound covers the complete current domain surface before
the constant is frozen; otherwise the bound is increased in this design before implementation.

## 4. The `mfm-catalog` port

`mfm-catalog` is a real persistence-port crate, not a transport crate. It owns:

- `ConfigName`: 1–64 lowercase ASCII letters/digits/hyphens, no leading/trailing hyphen;
- `ConfigDigest`: a `ContentDigest` wrapper that admits only `Sha256JcsV1`;
- opaque canonical `CatalogEntry`/`CatalogPage` custody records, insert outcomes, and conditional
  delete outcomes; neither record contains an entry-point id;
- distinct bounded/versioned `ConfigCursor` and `RunCursor` types;
- `ConfigCatalog` and `RunIndex` traits;
- `MemoryCatalog`; and
- catalog/index error vocabularies, including ambiguous mutation acknowledgement.

`mfm-app` owns `ConfigSummary`, `ImportOutcome`, and the public config views. For every read or list,
Application verifies each catalog row's digest and structurally decodes its canonical document,
then derives the checked `EntryPointId` for the summary. One corrupt row fails the request as
`InvalidCatalog`; there is no denormalized semantic column whose value can disagree with canonical
bytes. A page's bounded pure verification is one immediately awaited `spawn_blocking` call after
catalog IO; no IO or mutation authority enters that closure. The catalog remains mechanical custody
and never parses the config wire.

The `Store` trait remains unchanged. `Runtime` still receives only `Arc<dyn Store>`, so it cannot
enumerate runs or access configs structurally. A concrete store may separately implement
`RunIndex`; that never widens the authority of a `dyn Store` handle.

### Config lifecycle

| Action | Exact contract |
| --- | --- |
| Import absent name | Atomically insert; return `Created`. |
| Import same name and canonical document | Write nothing; return `Unchanged`. |
| Import same name, different canonical document | Write nothing; return `Conflict`. |
| Start `Current` | Atomically load and own the one `(name, digest, canonical)` snapshot currently bound to the name before planning or Store IO. |
| Start `Exact` | Perform the same atomic snapshot load and require its digest to equal the selected digest before planning or Store IO. |
| Delete | Atomically delete only when both name and caller-supplied digest match. Absence and digest mismatch are distinct; no run history is touched. |
| Ambiguous COMMIT | Return `Indeterminate`; the caller must re-read the name/digest to resolve the outcome. |

Names may be reused after deletion. They are mutable locators, never content identity. `Current`
deliberately means “the revision atomically observed now”; `Exact` is the revision assertion for
retries and schedulers, while delete always remains conditional on a digest. A concurrent
delete/re-import after either start selection cannot change the owned canonical snapshot used by
that call. There is no generic update or merge path and no transaction spanning catalog selection
and Runtime execution.

Deleting a config cannot damage a retained run: genesis already owns the exact Program and C0, and
Runtime `read`/`resume` never consults the catalog. No run→config annotation is added to Journal or
Store.

### PostgreSQL catalog

The catalog uses an independently gated `mfm_catalog` namespace and a fresh baseline file named for
that contract, not `0002` (which would falsely imply a run-history migration). Its marker is
`mfm.config-catalog-postgres.v1`.

```sql
CREATE SCHEMA mfm_catalog;

CREATE TABLE mfm_catalog.mfm_catalog_schema (
    schema_contract TEXT COLLATE "C" NOT NULL,
    CONSTRAINT mfm_catalog_schema_pkey PRIMARY KEY (schema_contract),
    CONSTRAINT mfm_catalog_schema_contract_check
        CHECK (schema_contract = 'mfm.config-catalog-postgres.v1')
);

INSERT INTO mfm_catalog.mfm_catalog_schema (schema_contract)
VALUES ('mfm.config-catalog-postgres.v1');

CREATE TABLE mfm_catalog.config_entries (
    config_name   TEXT COLLATE "C" NOT NULL,
    config_digest TEXT COLLATE "C" NOT NULL,
    canonical     BYTEA NOT NULL,
    CONSTRAINT mfm_catalog_config_entries_pkey PRIMARY KEY (config_name),
    CONSTRAINT mfm_catalog_config_entries_name_length_check
        CHECK (octet_length(config_name) BETWEEN 1 AND 64),
    CONSTRAINT mfm_catalog_config_entries_name_grammar_check
        CHECK (config_name ~ '^[a-z0-9][a-z0-9-]*$' AND right(config_name, 1) <> '-'),
    CONSTRAINT mfm_catalog_config_entries_digest_check
        CHECK (config_digest ~ '^content:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT mfm_catalog_config_entries_bytes_check
        CHECK (octet_length(canonical) BETWEEN 1 AND 262144)
);
```

The independent gate requires exactly that one marker row, the exact two logged tables, columns,
constraints, primary indexes, ownership/ACL posture, and no additional `mfm_` relation in the
`mfm_catalog` schema. It also proves the same PostgreSQL durability settings as the run gate. An
absent or partial namespace is incompatible; only the admin provisioner may install into a target
with no `mfm_catalog` schema.

The fixed catalog capacity is 256 live entries. Import serializes count/check/insert in PostgreSQL,
so CLI and daemon processes cannot race past the bound. Import and conditional delete use one
transaction, `synchronous_commit=on`, and explicit ambiguous-COMMIT classification. `256 × 256 KiB`
is a 64 MiB **live canonical-payload** ceiling, not a total disk ceiling: table/index/TOAST/WAL
overhead and deletion bloat remain.

Run-history and catalog gates remain independent. The simplest first implementation opens a
run-store/index handle and a catalog handle with their own gates; sharing a physical pool is an
optimization only if a combined composer proves both gates without making run-only Store use depend
on catalog health.

Exact ACL/ownership admission changes the current PostgreSQL run-store baseline and connection gate
even though its three table data shapes do not change. Existing owner-connected installations are
incompatible and provisioning refuses them; this clean-slate cut requires a fresh empty database
target and never migrates, re-owns, or rewrites retained history. The catalog begins at its one
current baseline with the same role posture.

## 5. Run enumeration

`RunIndex` projects only mechanical current-head data:

```rust
pub struct RunSummary {
    pub run_id: RunId,
    pub head_sequence: u64,
    pub head_digest: ContentDigest,
    pub total_bytes: u64,
}
```

`head_digest` comes from the head-frame foreign-key join; no frame parsing or semantic fold occurs.
There is deliberately no status, entry point, timestamp, config provenance, or derived failure
class. `run show` obtains semantic state one run at a time through Runtime.

Ordering is ascending `RunId` under Rust string order and PostgreSQL `TEXT COLLATE "C"`; config
ordering is ascending `ConfigName`. Pages use keyset traversal, never offsets. Cursors are bounded,
versioned, resource-discriminated, and include the fixed ordering. The exact wire is unpadded
base64url of canonical JSON and is at most 512 ASCII bytes:

```json
{"after":"alpha","order":"name-asc","resource":"configs","v":1}
```

The run form differs only as
`{"after":"<RunId>","order":"run-id-asc","resource":"runs","v":1}`. `after` is the last
returned checked identity and traversal is exclusive. A cursor from one resource/order cannot be
replayed against another; invalid base64url, noncanonical/unknown/duplicate fields, wrong version,
wrong resource/order, invalid identity, or overlength input is `invalid_cursor`. A page queries
`limit + 1` and emits `next_cursor` only when another row was observed.

Paging is **not a snapshot across requests**. Each page is one current statement snapshot. A row
inserted later and ordered before the cursor is not returned; config deletion can also remove an
unseen row. Static datasets are gap-free and duplicate-free. No concurrent snapshot guarantee is
claimed.

Changing the default order later is a wire change. A future recency view adds an explicit sort and a
separate non-authoritative projection; it does not silently reinterpret current cursors.

### Interoperable RunId generation

RunId generation is a client helper, never an Application or Runtime behavior. The algorithm is
`mfm.run-id.random.v1`:

1. obtain exactly 32 independently uniform octets from an operating-system cryptographic random
   source;
2. encode those octets as 64 lowercase hexadecimal characters;
3. construct the following exact UTF-8 JCS bytes, with no whitespace or trailing newline:
   `{"entropy":"<64 lowercase hex>","purpose":"mfm.run-id.random","v":1}`;
4. compute SHA-256 over those bytes; and
5. construct `run:sha256-jcs-v1:<64 lowercase digest hex>` from the 32 digest bytes.

The CLI fills the 32-byte array through a direct, pinned OS-random dependency and fails closed on
entropy-source failure. It never falls back to time, process id, counters, environment, machine
identity, config content, or provider data. `mfm-app` exposes the pure, IO-free
`derive_run_id([u8; 32]) -> RunId` helper used by the CLI and its fixtures; Application methods do
not call it. The direct dependency is justified because fresh caller authority must not depend on a
transitive RNG API or an ad-hoc uniqueness recipe. Non-Rust clients implement the five frozen steps
and verify against both vectors:

```text
entropy = 0000000000000000000000000000000000000000000000000000000000000000
canonical =
{"entropy":"0000000000000000000000000000000000000000000000000000000000000000","purpose":"mfm.run-id.random","v":1}
run_id = run:sha256-jcs-v1:1ffc529adb99fb0f91d2c1712affb64a6d57fcdc9ac78c31c9681449e9c9f79a

entropy = 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
canonical =
{"entropy":"000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f","purpose":"mfm.run-id.random","v":1}
run_id = run:sha256-jcs-v1:4412ec646bc21fee7eb806a97dccf2ec412ce38b33f9c3d7ea25e83fac13339e
```

The RunId wire reveals no entropy preimage, so Application can enforce only the existing checked
identity grammar; official client helpers and client conformance tests enforce generation
provenance. Explicit IDs remain accepted for interoperability and retries.

The fixed-entropy vectors prove derivation and do not authorize deterministic entropy in
production. Generation does no existence preflight: uniqueness comes from 256 random bits, while a
collision is handled by Runtime's existing exact-admission rule—converging only for the same
Program/C0 and returning `RunAdmissionConflict` otherwise—rather than a racy list/read check.

## 6. `mfm-app`: one typed use-case surface

Application remains one non-generic struct and preserves injected construction for library users
and hermetic tests. `open` is a product convenience over the same constructor, not the only way to
obtain an Application. An opaque `ComposedRuntime` packages one Runtime with the exact public
binding set from which its immutable assembly was built, so injected construction cannot pair two
unrelated truths.

Its only constructor accepts one `Arc<B>` where `B: Store + RunIndex`, plus checked typed adapter
bindings, builds the assembly, and privately coerces that same backend into Runtime and RunIndex
handles. The production PostgreSQL run backend and `MemoryStore` implement both ports. This also
prevents `run list` from silently enumerating a different repository than `start/read/progress`.
`BoundCapabilitySet` is opaque and derives each `PublicBindingView` from the same typed target and
provider handle it consumes; no caller supplies a second view. Catalog custody remains independently
injectable.

```rust
pub enum ConfigSelection {
    Current { name: ConfigName },
    Exact { name: ConfigName, digest: ConfigDigest },
}

pub struct StartRunResult {
    pub config: ConfigSummary,
    pub run: RunView,
}

pub enum RunRecovery {
    Start { run_id: RunId, config: ConfigSummary },
    Progress { run_id: RunId },
}

pub enum RunRequestError {
    Request(RequestError),
    AppendIndeterminate { recovery: RunRecovery },
}

pub struct ComposedRuntime { /* private Runtime plus exact PublicBindingSet */ }
pub struct Application { /* ComposedRuntime, RunIndex, ConfigCatalog */ }

impl Deployment {
    pub async fn load(override_path: Option<&std::path::Path>)
        -> Result<Self, ComposeError>;
}

impl ComposedRuntime {
    pub fn compose<B>(
        backend: Arc<B>,
        bindings: BoundCapabilitySet,
    ) -> Result<Self, ComposeError>
    where
        B: Store + RunIndex + 'static;
}

impl Application {
    pub fn from_parts(
        composed: ComposedRuntime,
        catalog: Arc<dyn ConfigCatalog>,
    ) -> Result<Self, ComposeError>;

    pub async fn open(deployment: &Deployment) -> Result<Self, ComposeError>;

    pub const fn entry_points() -> &'static [EntryPointSummary];
    pub fn bindings(&self) -> &[PublicBindingView];

    pub async fn import_config(
        &self,
        name: ConfigName,
        document: ConfigDocument,
    ) -> Result<ImportOutcome, RequestError>;
    pub async fn read_config(&self, name: &ConfigName)
        -> Result<StoredConfigView, RequestError>;
    pub async fn list_configs(&self, page: &ConfigPageRequest)
        -> Result<ConfigPage, RequestError>;
    pub async fn delete_config(&self, name: &ConfigName, digest: &ConfigDigest)
        -> Result<(), RequestError>;

    pub async fn start_run(&self, run_id: RunId, config: &ConfigSelection)
        -> Result<StartRunResult, RunRequestError>;
    pub async fn progress_run(&self, run_id: &RunId)
        -> Result<RunView, RunRequestError>;
    pub async fn read_run(&self, run_id: &RunId)
        -> Result<RunView, RequestError>;
    pub async fn list_runs(&self, page: &RunPageRequest)
        -> Result<RunPage, RequestError>;
}
```

`ConfigSelection` is the sole shared start selector. `Current` and `Exact` both produce one
validated, owned `ConfigSummary` plus its canonical bytes from the same catalog snapshot; only
`Exact` compares a caller assertion. `StartRunResult` returns that resolved summary alongside the
`RunView`, so both transports expose the actual digest even when the caller selected `Current`.

Every Application call and the underlying `Runtime::start` receive the already constructed RunId.
Neither layer owns randomness or an optional/server-generated identity branch. `RunRequestError`
wraps ordinary data-free `RequestError` values and carries checked public recovery identity only for
an ambiguous run append. Its exhaustive `RunRecovery` sum avoids a nullable config field:
`Start` retains the resolved config summary, while `Progress` needs only its already explicit RunId.

Schema provisioning is a separate CLI-only composition entry, not a method on listener-held
Application state. `Application::start_portfolio`, `resume`, `read`, and the one-target
`portfolio_assembly` are deleted in the same cut that rewrites their CLI callers.

`Application::entry_points()` is an associated function because compiled entry points do not depend
on a deployment. The CLI can therefore call the same contract without constructing ambient
authority; the REST daemon calls it after startup like any other discovery function.
`EntryPointSummary` borrows the trusted compiled `&'static str`; a contract test parses every listed
constant as `EntryPointId`. The slice is strictly ascending by EntryPointId, matching CLI/REST
output. Retained/imported `ConfigSummary` still owns a checked `EntryPointId`.

Entry-point generalization is one private exhaustive enum match. No planner registry or erased C0 is
introduced: `Runtime::start<T: MfmValue>` continues to receive its exact typed admitted context.
Adding an entry point changes its domain planner, one enum variant/match arm, the static entry-point
list, assembly registrations, and tests. Neither binary's route/command code changes.

## 7. Errors and run rendering

There are four disjoint failure owners:

- `ComposeError`: local startup/deployment/environment/provider/store/catalog/assembly failures.
  It can reach local stderr only, before REST binds.
- Checked input errors: shared constructors for config names/digests/documents, RunIds, cursors, and
  page requests. They remain separate from Application so typed method signatures have no
  unreachable invalid-scalar variants.
- `RequestError`: data-free, redaction-safe shared use-case failures. It owns stable `code()` and
  `Display`; both transports render the same code/message for these variants. `RunRequestError`
  wraps it and adds only the frozen, public `RunRecovery` payload when append acknowledgement is
  ambiguous.
- CLI/REST boundary errors: argv/usage or HTTP media type, body framing, fallback 404/405, local
  busy/deadline, Host/Origin, and pre-routing server rejection. They never create unreachable
  Application error variants.

Every Runtime variant has an explicit shared mapping, including `IncompatibleAssembly`. Catalog
mutation `Indeterminate` is not collapsed into unavailable, and Runtime append `Indeterminate` maps
only to `RunRequestError::AppendIndeterminate`. Adding a shared variant causes compile errors in
both transport mappings.

The complete checked-input rendering is:

| Condition | Code | Exact message | HTTP |
| --- | --- | --- | --- |
| Invalid config name | `invalid_config_name` | `config name is invalid` | 400 |
| Invalid config digest | `invalid_config_digest` | `config digest is invalid` | 400 |
| Invalid RunId | `invalid_run_id` | `run id is invalid` | 400 |
| Invalid/foreign/oversize cursor | `invalid_cursor` | `cursor is invalid` | 400 |
| Page limit outside 1–200 | `invalid_page_limit` | `page limit is invalid` | 400 |
| Invalid JSON/UTF-8/number/depth/duplicate/float | `malformed_config_document` | `config document is malformed` | 400 |
| Unknown/schema/domain document failure | `invalid_config_document` | `config document is invalid` | 422 |
| Encoded config above fixed bound | `config_document_too_large` | `config document is too large` | 413 |
| Blocking document task fails to join | `internal` | `application internal failure` | 500 |

The complete `RequestError` rendering is:

| Variant | Code | Exact message | HTTP |
| --- | --- | --- | --- |
| `ConfigAbsent` | `config_absent` | `config is absent` | 404 |
| `ConfigConflict` | `config_conflict` | `config name is bound to different content` | 409 |
| `ConfigDigestMismatch` | `config_digest_mismatch` | `config digest does not match current content` | 409 |
| `InvalidConfigDocument` | `invalid_config_document` | `config document is invalid` | 422 |
| `CatalogCapacity` | `config_catalog_capacity` | `config catalog capacity exceeded` | 422 |
| `CatalogIndeterminate` | `config_mutation_indeterminate` | `config mutation outcome is indeterminate` | 503 |
| `InvalidCatalog` | `invalid_config_catalog` | `retained config catalog is invalid` | 500 |
| `RunAbsent` | `run_absent` | `run is absent` | 404 |
| `RunAdmissionConflict` | `run_admission_conflict` | `run admission conflicts with retained history` | 409 |
| `InvalidRunHistory` | `invalid_run_history` | `retained run history is invalid` | 500 |
| `IncompatibleAssembly` | `incompatible_assembly` | `runtime assembly is incompatible` | 500 |
| `RunCapacity` | `run_capacity` | `run capacity exceeded` | 422 |
| `BindingUnbound` | `binding_unbound` | `required capability binding is unavailable` | 409 |
| `InvalidRunIndex` | `invalid_run_index` | `retained run index is invalid` | 500 |
| `DependencyUnavailable` | `dependency_unavailable` | `application dependency is unavailable` | 503 |
| `Internal` | `internal` | `application internal failure` | 500 |

`RunRequestError::Request` delegates to that table. Its one additional variant is:

| Variant | Code | Exact message | HTTP |
| --- | --- | --- | --- |
| `AppendIndeterminate { recovery }` | `run_append_indeterminate` | `run append outcome is indeterminate` | 503 |

Normal shared errors have exactly `{"code":"...","message":"..."}`. The indeterminate variant
instead has the exhaustive shape
`{"code":"run_append_indeterminate","message":"run append outcome is indeterminate",`
`"recovery":RunRecovery}` where:

```text
RunRecovery = {"kind":"start", "run_id":RunId, "config":ConfigSummary}
            | {"kind":"progress", "run_id":RunId}
```

CLI and REST serialize this same recovery object. For an indeterminate generated-ID CLI start, it
is the stable automation handoff: the caller uses `run show --run-id <ID>` and, if a retry is
required, the recovered digest selects `Exact` with the same RunId. Mechanical run listing is never
used to rediscover an identity whose append acknowledgement was ambiguous.

REST adds only these routed boundary codes; CLI never grows unreachable arms for them:

| Condition | Code | Exact message | HTTP |
| --- | --- | --- | --- |
| Invalid/unexpected action body | `invalid_request_body` | `request body is invalid` | 400 |
| Unknown/duplicate/malformed list query field | `invalid_query` | `request query is invalid` | 400 |
| Host is not exactly `mfm.local` | `invalid_host` | `request host is invalid` | 400 |
| Any Origin header | `origin_forbidden` | `request origin is forbidden` | 403 |
| Router fallback | `route_not_found` | `route is not found` | 404 |
| Wrong routed method | `method_not_allowed` | `method is not allowed` | 405 |
| Route-specific body ceiling | `request_body_too_large` | `request body is too large` | 413 |
| Wrong/missing JSON media type | `unsupported_media_type` | `content type must be application/json` | 415 |
| Missing delete digest header | `precondition_required` | `config digest header is required` | 428 |
| Same RunId already active | `run_busy` | `run already has an active request` | 409 |
| Global run semaphore full | `too_many_requests` | `run request limit is reached` | 429 |
| REST run deadline expires | `deadline_exceeded` | `run request deadline exceeded` | 504 |

CLI adds one boundary failure that is impossible after a RunId exists:

| Condition | Code | Exact message | Exit |
| --- | --- | --- | --- |
| OS entropy source fails | `run_id_generation_failed` | `run id generation failed` | 2 |

CLI exit behavior remains:

| Exit | Meaning |
| --- | --- |
| 0 | Non-run command succeeded, or a run view is `Succeeded`. |
| 1 | A run view is `Runnable` or durably `Failed`. |
| 2 | No run view: usage, composition, boundary, or request error. |

REST clients branch on the stable JSON `code`, not the message or coarse HTTP class. Shared absence
is 404, malformed input is 400, media type is 415, start/admission/binding conflict is 409, size is
413, unplannable input/capacity is 422, process pressure is 429, deadline is 504, dependency or
indeterminate state is 503, and invalid retained/internal state is 500. A missing required delete
digest header is 428. A durably failed **run** is still a successful HTTP request and returns 200
with a `Failed` state.

### Shared result models

The JSON field model is frozen independently of CLI text formatting. `EntryPointList`,
`BindingList`, `ImportOutcome`, `StoredConfigView`, `ConfigPage`, `StartRunResult`, and `RunPage`
have exactly these structural shapes; no timestamp, status, deployment resolver, or correlated
optional field is added:

```text
EntryPointList   = {"items":[{"entry_point": EntryPointId}]}
BindingList      = {"items":[{"kind":"evm",
                               "chain_id": u64,
                               "endpoint_id": string,
                               "binding_ref": ContentRef}]}
ConfigSummary    = {"name": ConfigName,
                    "digest": ConfigDigest,
                    "entry_point": EntryPointId}
ImportOutcome    = {"outcome":"created"|"unchanged", "config": ConfigSummary}
StoredConfigView = {"config": ConfigSummary, "document": RawCanonicalJson}
ConfigPage       = {"items":[ConfigSummary], "next_cursor": ConfigCursor|null}
StartRunResult   = {"config":ConfigSummary, "run":RunView}
RunPage          = {"items":[RunSummary], "next_cursor": RunCursor|null}
```

`RunSummary` has exactly `run_id`, `head_sequence`, `head_digest`, and `total_bytes` as defined
above. `ContentRef` uses its existing `{schema_id, content_digest}` representation. `BindingList`
is an exhaustive tagged sum: a future non-EVM capability adds a variant rather than borrowing EVM
fields. The canonical config document and terminal value occupy one raw JSON value position, not a
quoted JSON string.

`StartRunResult.run.run_id` is the explicit or CLI-generated identity passed to Application;
`StartRunResult.config.digest` is the revision actually selected. Both are present for `Current`
and `Exact`. A successful CLI text rendering includes `run_id`, `config_name`, `config_digest`, and
`entry_point` lines before the ordinary run-state fields. Its JSON rendering is exactly
`StartRunResult`.

### Run JSON preserves the sum

```json
{
  "run_id": "run:sha256-jcs-v1:<64 hex>",
  "head_sequence": 11,
  "head_digest": "content:sha256-v1:<64 hex>",
  "state": {
    "kind": "succeeded",
    "contract_ref": {
      "schema_id": "schema:...",
      "content_digest": "content:sha256-v1:..."
    },
    "value_ref": {
      "schema_id": "schema:...",
      "content_digest": "content:sha256-v1:..."
    },
    "value": {}
  }
}
```

Runnable state is exactly `{"kind":"runnable"}`. Failed state has the same terminal fields with
`kind:"failed"`. REST embeds `RetainedValueView::canonical_bytes()` verbatim with
`serde_json::value::RawValue`; enabling serde_json's `raw_value` feature is part of the manifest
cut. It never parses and reserializes retained bytes.

CLI gains `--output text|json`, default `text`. JSON uses this exact model, which is the managed
cross-transport comparison and stable automation surface. Text is explicitly human presentation,
not a parseable compatibility contract; scripts must select JSON. It remains line-oriented and
includes terminal contract ref, value ref, and exact canonical value, so human mode does not hide
information present in REST. An indeterminate run mutation renders the shared recovery object in
JSON on stderr. Text mode prints its `recovery.kind`, `recovery.run_id`, and all start-config summary
fields when present, so a generated RunId is recoverable in either output mode.

## 8. CLI grammar

`entry-point list` is static, does not accept `--deployment`, and never resolves Deployment. Its
items are the exact valid `entry_point` document tags; it is discovery for config authors, not a
second start selector. Every other command accepts an optional
`--deployment <PATH>` override; without it, the shared XDG/HOME resolver loads the conventional
`deployment.toml` path above.

```text
mfm_cli [--output text|json] entry-point list

mfm_cli [--deployment <PATH>] [--output text|json] store init \
    --admin-store-locator-env <NAME>
mfm_cli [--deployment <PATH>] [--output text|json] binding list

mfm_cli [--deployment <PATH>] [--output text|json] config import <NAME> --from <PATH|->
mfm_cli [--deployment <PATH>] [--output text|json] config list [--cursor <C>] [--limit <N>]
mfm_cli [--deployment <PATH>] [--output text|json] config show <NAME>
mfm_cli [--deployment <PATH>] [--output text|json] config delete <NAME> --digest <DIGEST>

mfm_cli [--deployment <PATH>] [--output text|json] run start --config <NAME> \
    [--config-digest <DIGEST>] [--run-id <RUN_ID>]
mfm_cli [--deployment <PATH>] [--output text|json] run progress --run-id <RUN_ID>
mfm_cli [--deployment <PATH>] [--output text|json] run show --run-id <RUN_ID>
mfm_cli [--deployment <PATH>] [--output text|json] run list [--cursor <C>] [--limit <N>]
```

`run start` accepts no `--entry-point`. Without `--config-digest`, the CLI constructs
`ConfigSelection::Current { name }`; with it, the CLI constructs
`ConfigSelection::Exact { name, digest }`. Without `--run-id`, the CLI obtains 32 bytes from the OS
cryptographic random source, calls the frozen pure derivation helper, and passes that fresh RunId to
Application exactly as though the user supplied it. An explicit `--run-id` is parsed without
generation and supports idempotent retries, schedulers, and cross-process coordination. A caller
that needs revision-stable retry supplies both `--run-id` and `--config-digest`; retrying `Current`
intentionally re-observes the name if no admission was retained.

Random-source failure occurs before the Application call and has no recovery identity. After a
generated RunId exists, the CLI retains it through the call and includes it in every successful
`StartRunResult` or `RunRecovery::Start` indeterminate error. Ordinary determinate start failures
need no recovery field because no ambiguous append occurred.

`--from -` reads stdin through `MAX_CONFIG_DOCUMENT_BYTES + 1`; a regular file is also read
through the same bound before allocation. A private key, mnemonic, passphrase, database URL, or RPC
URL is never accepted on argv or emitted.

JSON config/run/list results are the same shapes REST returns. Text formatting is documented with
examples in the current CLI README but may evolve as human presentation; required recovery fields,
exit meanings, and JSON do not. Shared request errors render as `error: <message>` in text mode and
the two-field error object on stderr in JSON mode, except the frozen indeterminate recovery sum
above; clap's own usage diagnostics remain CLI-local.

The old `init | snapshot | show --config` grammar and combined config document are deleted outright.

## 9. REST grammar

The daemon starts as:

```text
mfm_rest_api serve [--deployment <PATH>] \
    --unix-socket <PATH> [--recover-stale-socket] \
    [--max-in-flight-runs <N>] [--run-timeout <SECONDS>]
```

It resolves the override or conventional `deployment.toml` path and fully composes Application
before binding. `ComposeError` prints one reviewed local stderr line
and exits 2; it has no HTTP mapping. The socket parent is opened without following links and must be
a directory owned by the effective user with no group/other permission bits; it must grant the owner
read, write, and search. All leaf operations are relative to that held directory descriptor. The
socket leaf must initially be absent and its checked encoded path must fit the platform limit. Bind
creates a `0600` socket; the daemon never unlinks an unexpected existing path. Requests require the
exact `Host: mfm.local` authority and no `Origin` header.

For socket leaf `<name>`, a sibling `<name>.mfm-rest.lock` regular file is opened without following
links as owner-only and held under an exclusive nonblocking lock for the daemon lifetime. After bind,
the daemon rewrites and fsyncs a versioned marker containing the socket's device/inode, then fsyncs
the directory; a torn/invalid marker is safe refusal, never recovery authority. Graceful shutdown
unlinks only when a fresh relative metadata check still
matches that recorded socket, fsyncs the directory, clears/fsyncs the marker, and leaves the locked
file for reuse. After a crash, default startup refuses the existing leaf; `--recover-stale-socket`
removes it only while holding the lock, when the marker and current owner/type/mode/device/inode all
match and a stream connect returns Linux `ECONNREFUSED`. It then fsyncs the directory before bind.
Any successful/inconclusive connect, missing/mismatched marker, foreign file, symlink,
ownership/mode failure, or changed inode is refused, never removed.

The implementation pins Axum 0.8.9 with its Tokio/HTTP/1 support and passes
`tokio::net::UnixListener` directly to `axum::serve`; Axum's `Listener` implementation removes the
need for a security-critical custom accept loop or direct Hyper/hyper-util dependencies. The Axum
`Router` remains the sole routing/extraction/rendering implementation, HTTP/2 is not enabled, and
there is no dormant TCP listener path. Graceful-shutdown, stale-path, permission, accept-failure,
and connection tests cover the listener lifecycle.

| Method | Path | Request | Success |
| --- | --- | --- | --- |
| GET | `/healthz` | none | 200 `{"status":"ok"}` |
| GET | `/v1/entry-points` | none | `EntryPointList` |
| GET | `/v1/bindings` | none | `BindingList` |
| PUT | `/v1/configs/{name}` | raw config document | 201 Created / 200 Unchanged, `ImportOutcome` + `Location` |
| GET | `/v1/configs` | `cursor`, `limit` | `ConfigPage` |
| GET | `/v1/configs/{name}` | none | `StoredConfigView` |
| DELETE | `/v1/configs/{name}` | required `MFM-Config-Digest` header | 204 |
| GET | `/v1/runs` | `cursor`, `limit` | mechanical `RunPage` |
| POST | `/v1/runs/{run_id}/start` | tagged `ConfigSelection` below | 200 `StartRunResult` |
| POST | `/v1/runs/{run_id}/progress` | strict empty JSON object | 200 `RunView` |
| GET | `/v1/runs/{run_id}` | none | 200 `RunView` |

`/healthz` is explicitly transport liveness, not an Application entry point. There is no REST
schema-provisioning route, deployment view, inline run document, server-derived RunId, config update,
keystore route, or placeholder replay/trace/audit/export/effect route.

REST clients always generate and retain the checked `{run_id}` before issuing the mutating request;
the daemon never calls the RunId derivation helper. The strict start body is exactly one of:

```json
{"config":{"kind":"current","name":"daily"}}
{"config":{"kind":"exact","name":"daily","digest":"content:sha256-jcs-v1:<64 hex>"}}
```

This wire is the exhaustive `ConfigSelection` sum, not a nullable digest field. Missing/unknown
`kind`, a digest on `current`, a missing digest on `exact`, duplicate/unknown fields, and any
`entry_point` field are `invalid_request_body`. The entry point remains selected solely by the
stored tagged document. A successful response includes the selected `ConfigSummary` and RunId in
`StartRunResult`; an ambiguous append includes the same shared `RunRecovery::Start` object as CLI.
Retrying/scheduled REST clients reuse the path RunId and send the recovered or previously observed
digest through `exact`.

DELETE carries the bare checked `ConfigDigest` in the dedicated `MFM-Config-Digest` header. A
missing header is 428 `precondition_required`, malformed input is 400 `invalid_config_digest`, an
absent name is 404, and an atomically observed digest mismatch is the same shared
`config_digest_mismatch` code/message and 409 status as CLI `Exact`/REST `exact` start. No endpoint emits ETag: PUT
canonicalizes its submitted representation, and every relevant digest already appears in the body.
Both successful PUT outcomes set `Location: /v1/configs/{name}`.

All mutating requests require the `application/json` media type, optionally with only a UTF-8
charset parameter, except bodyless conditional DELETE, which requires the custom digest header.
The daemon emits no permissive CORS headers and normalizes all routed Axum rejections, fallbacks,
and method failures into exactly `{"code":"...","message":"..."}`. Shared indeterminate run
errors use the one frozen recovery-envelope sum above. HTTP parser failures that occur before Axum routing
(for example an overlarge request line) are explicitly outside that JSON contract.

Axum's implicit HEAD behavior is retained deliberately for every GET route: HEAD performs the same
admission/Application read, returns the GET-equivalent status and headers, and omits the body.
Run HEAD is therefore covered by the global run semaphore/deadline. HEAD error responses also omit
their JSON body as required by HTTP semantics; every other unlisted method receives the normalized
405 envelope.

Config/run bodies use `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`. No request or
response body, header, URL value, deployment value, or canonical document enters tracing/access
logs. Every non-204 success and every routed error uses `Content-Type: application/json`; 204 has
no body or content type.

### Surface parity matrix

| Use case | Shared function | CLI | REST |
| --- | --- | --- | --- |
| List compiled entry points | `Application::entry_points` | `entry-point list` | `GET /v1/entry-points` |
| List bound capabilities | `Application::bindings` | `binding list` | `GET /v1/bindings` |
| Import a config | `Application::import_config` | `config import` | `PUT /v1/configs/{name}` |
| List configs | `Application::list_configs` | `config list` | `GET /v1/configs` |
| Read a config | `Application::read_config` | `config show` | `GET /v1/configs/{name}` |
| Conditional config delete | `Application::delete_config` | `config delete --digest` | `DELETE /v1/configs/{name}` + digest header |
| Start a run | `Application::start_run` with explicit RunId and `ConfigSelection` | `run start`; optional CLI-only generation | `POST /v1/runs/{run_id}/start`; client-generated id |
| Progress a run | `Application::progress_run` | `run progress` | `POST /v1/runs/{run_id}/progress` |
| Read a run | `Application::read_run` | `run show` | `GET /v1/runs/{run_id}` |
| List run heads | `Application::list_runs` | `run list` | `GET /v1/runs` |
| Provision schemas | separate provisioning adapter | `store init` | absent by authority rule |
| Listener liveness | REST boundary only | absent | `GET /healthz` |
| Keystore administration | future custody port | future CLI-only RFC | absent by custody rule |

## 10. REST safety and admission control

| Control | Contract |
| --- | --- |
| Config body | shared 256 KiB limit at extractor and checked-constructor boundaries |
| Run action body | 4 KiB extractor ceiling; exhaustive start object or a deserialized empty object with unknown fields denied for progress |
| Bodyless routes | bounded byte extractor accepts exactly zero bytes |
| Matched path scalars | fixed encoded pre-parse lengths; decoding/constructor failure maps to that route's checked config-name or RunId error |
| List query | 1 KiB raw ceiling; only one each of `cursor` and `limit`; strict decoding before checked constructors |
| Page size | 1–200, default 50 |
| Config catalog | 256 live entries, enforced transactionally in PostgreSQL |
| Global run requests | fixed bounded semaphore covers start, progress, and show |
| Same-run requests | removable bounded active set rejects concurrent HTTP start/progress for one RunId |
| Run deadline | REST-local bounded deadline; expiry is 504 and cancellation leaves the run resumable |

The active set and semaphore are **process-local amplification controls**, not a global scheduler.
The CLI and a second daemon can bypass them; Store correctness still converges because Reads are
duplicate-safe and appends are atomic, but provider IO may be duplicated. A durable cross-process
lease would be a new scheduling contract and is not smuggled into Store.

Each accepted fresh RunId can append genesis, run history has no delete, and the run catalog has no
quota. Sequential callers can grow the database without bound. A valid `run show` may load up to the
512 MiB format ceiling. These are authority/resource risks, not solved by per-request limits.

The Unix socket makes “single user” an OS-enforced local boundary; it is still not authentication
against code running as that user or root. There is no TCP bind option in v1. Adding TCP later
requires a separately reviewed authenticated authority model rather than a documentation flag.

## 11. Keystore: long-term surface, separate custody RFC

No key command or route ships in this client-surface wave.

The current `Keystore` dies with one CLI process and has no authenticated at-rest representation.
Adding `key import`, `key list`, or `key delete` now would be either a no-op across invocations or an
unreviewed plaintext format. Existing `!Send + !Sync` tests prove thread affinity only; they do not
prove an HTTP handler cannot receive secret bytes or command a future `Send + Sync` signer handle.

The later custody RFC may define CLI-only conceptual actions for import, authenticated list, and
conditional delete, but their grammar is not reserved here. It must land atomically with:

- exact supported key algorithms/encodings and key ids derived from validated public material;
- a bounded whole-file authenticated-encryption format;
- Argon2id with reviewed, benchmarked parameters, salt, nonce, and domain-separated AAD;
- passphrase authentication for **import, list, and delete** (an authenticated header cannot be
  verified or rewritten without a key);
- bounded parsing before KDF allocation, zeroization, and immediately awaited pure blocking KDF;
- no-follow regular-file opens, ownership/mode/parent checks, inter-process locking, atomic durable
  replace, and crash tests; and
- honest deletion semantics: logical removal and key-buffer zeroization, not guaranteed physical
  erasure from COW filesystems, SSDs, snapshots, or backups.

Secret and passphrase ingress is a local file descriptor (`--from <PATH|->`, tty, or passphrase
file), never argv or environment. REST has no import, list, delete, unlock, or export route under
the no-auth design. A future execution surface may select a pre-admitted public signer alias through
a narrow `Signer` handle; that handle must expose neither keystore administration nor raw secrets.

Mechanical enforcement is dependency-shaped: REST and the shared Application request enum do not
depend on keystore administration or secret parsers. `mfm-signing::Signer`, when implemented, is a
separate execution capability.

---

# Architecture contract changes for the implementation wave

`docs/architecture.md` gains these rules in the commits that introduce them:

### Symmetry

> CLI and REST render one typed Application use-case surface. Binaries own bounded transport
> parsing/rendering and transport policy only. Symmetry is the default; every asymmetry is named
> with the invariant it protects in the owning binary README.

### Exposure

> A shared Application request contains only bounded, secret-free data and stable selectors for
> pre-bound capabilities. It cannot introduce environment resolution, a filesystem/network locator,
> secret custody, schema authority, or an unbounded durable effect. REST exposure additionally
> requires a bounded transport rendering and explicit local trust assumptions.

### Responsibility table

| Owner | Responsibility | Must not own |
| --- | --- | --- |
| Store | complete-prefix load and atomic exact-head append | enumeration, Program/State/capability/reducer/config semantics |
| Catalog/index port | conditional named custody of opaque canonical config bytes; mechanical current-head enumeration | config-wire parsing, run folds/status, Program semantics, config merging, run→config authority |
| Transport-security primitive | checked TLS-root specs and exact immutable root-store loading | endpoint URLs, protocol clients, locator resolution, credentials |
| Application | injected and live composition; typed config/run/discovery use cases; exhaustive entry-point planning | sockets, argv/HTTP, sessions, frame inspection, status derivation, secret administration |
| Binaries | bounded transport parsing, client-side RunId entropy where offered, one Application call, transport policy, redacted rendering | composition, domain planning, environment resolution, execution lifecycle, run semantics |

### Named asymmetries

| Asymmetry | Reason |
| --- | --- |
| `store init` is CLI-only and outside listener-held Application state | A network caller must not provision schemas. |
| Keystore administration is CLI-only in its future custody RFC | Secrets/passphrases and irreversible custody mutation do not cross unauthenticated HTTP. |
| REST owns liveness, Host/Origin/media checks, backpressure, and deadline | These are listener policies, not use-case outcomes. |
| CLI may generate a RunId when omitted; REST requires it in the path | Generation is client convenience completed before the shared call; Application and Runtime always receive explicit identity. |
| CLI exit 1 reflects Runnable/Failed; HTTP returns 200 for a durably Failed run | Shell status represents run outcome; HTTP status represents request outcome. |

---

# Complete cutover and deletion scope

Deleted outright in the implementation wave:

- CLI `init | snapshot | show --config` grammar and its combined config schema;
- `CliConfig`, `EvmRouteConfig`, `StoreConfig`, CLI environment resolution, concrete composition,
  Application error adapter, and ad-hoc output DTO logic;
- `Application::start_portfolio`, `resume`, and `read` public names;
- the mandatory-only `ConfigRef` start shape and any start-time entry-point selector;
- one-target `portfolio_assembly`;
- REST unavailable stub;
- `MemoryStore`'s `HashMap` run index;
- the one-route/no-resume/REST-stub known-gap entries; and
- domain/live/storage dependencies from the CLI binary that move behind `mfm-app`.

Added/changed:

- strict XDG/HOME-resolved `deployment.toml` bootstrap with an optional argv override and no
  `$MFM_DEPLOYMENT` or locator values in the file;
- new `mfm-catalog` and `mfm-transport-security` crates with workspace/model placement;
- `mfm-store` implements the separate `RunIndex` port without changing `Store`;
- PostgreSQL implements independently gated config custody and mechanical run paging;
- `mfm-app` gains config/deployment/failure modules, the strict TOML/XDG bootstrap loader, injected
  construction, and live composer;
- `mfm-app` owns `ConfigSelection`, `StartRunResult`, indeterminate `RunRecovery`, and the pure
  interoperable RunId derivation helper while both Application and Runtime retain explicit RunIds;
- Portfolio config rustdoc/README stop calling material process-local;
- EVM composition accepts all Deployment-declared binding mappings;
- provider/PostgreSQL transport authority is explicit and TLS-capable;
- CLI and REST READMEs become their complete current contracts;
- `docs/design.md`, `docs/architecture.md`, `docs/persisted-public-surfaces.md`, and
  `docs/known-gaps.md` change with their owning code commits; and
- Nixfied gains managed transport-authority and REST/cross-transport e2e tasks.

The Journal frame wire and run-history table/column data shape do not change. The PostgreSQL
baseline, ownership/ACL posture, and gate do change as stated above; old installations are rejected.

---

# Verification design

Tests land with the boundary they prove.

1. **Config document contract** — duplicate/unknown/floating/oversize rejection; hostile ambient
   field corpus; singular/missing/unsorted/duplicate routes rejected; selector/quote/route coverage
   planned at import; `InvalidValue` renders 422 while trusted `InvalidContinuation`/`Program`
   failures render 500; key reordering canonicalizes identically; config digest is exactly
   `sha256-jcs-v1`; representative Program/C0 fixtures stay frozen. The compiled entry-point list
   is exactly the accepted document-tag set, and an unknown tag fails import rather than becoming a
   start-time choice.
2. **Worst-case config bound** — generate maximum accepted current Portfolio public strings,
   routes, collections, and 64 total sources; prove its canonical bytes fit the selected bound.
3. **Catalog contract** — Created/Unchanged/Conflict; atomic 256-entry admission across concurrent
   callers; compare-and-delete; delete/re-import ABA resistance; ambiguous import/delete COMMIT;
   each read returns one internally consistent owned `(name, digest, canonical)` snapshot across a
   concurrent delete/re-import; canonical/digest disagreement rejected on load; no semantic
   entry-point column; Application derives the checked entry point from every returned document;
   static page ordering/cursors.
4. **Run index contract** — Memory/PostgreSQL identical static pages with four exact fields; cursor
   type/version rejection; no status parsing; explicitly demonstrate a concurrent earlier-key insert
   may be missed rather than asserting snapshot pagination or generated-RunId recovery.
5. **Schema isolation** — installed `mfm_catalog` is invisible to all `public` run gates; either
   schema may be incompatible without resetting the other; both provisioning entries are
   idempotent and never migrate. Provisioning rejects unequal admin/runtime targets; the runtime
   role passes both gates and DML tests but cannot create, alter, drop, or own either schema/table.
6. **Application contract** — multi-route assembly/start; bound/unbound route mapping before Store
   IO, including an otherwise valid upper-half-`u64` config that TOML cannot bind;
   immediately-awaited blocking planning on import and start; caller-invalid versus trusted
   planner mapping. `Current` selects one atomic snapshot and returns its digest; `Exact` accepts the
   same revision and rejects a mismatch before Store IO; a rebind after selection cannot change
   either call. Both paths pass the caller's exact RunId to Runtime and return `StartRunResult`; no
   Application request accepts an entry point or optional RunId. Start/progress append ambiguity
   maps exhaustively to the corresponding `RunRecovery` shape, including generated-id and resolved
   digest recovery for start. Deleting a config never damages retained read/progress; every Runtime
   and catalog error maps exhaustively to the frozen code/message table; injected Memory
   construction needs no PostgreSQL and cannot pair Runtime/RunIndex/binding sets from different
   composers.
7. **RunId generation contract** — both frozen entropy/preimage/digest vectors pass through the pure
   helper and an independent test implementation; explicit IDs bypass entropy completely; an
   injected entropy source proves exactly 32 bytes are consumed and failure maps to the CLI-only
   code before Application. Source/dependency review proves there is no time/PID/counter/ambient
   fallback or existence preflight. CLI renderer fixtures prove successful generated starts expose
   RunId plus resolved digest and an indeterminate start emits the same recoverable RunId/config
   summary in text and JSON.
8. **Deployment bootstrap** — strict TOML accepts only the exact tables above; malformed UTF-8,
   duplicate/unknown keys or tables, include/profile/interpolation attempts, invalid environment
   names, zero/negative/non-integer/out-of-range chain ids, unsorted/duplicate/257 routes, and
   encoded oversize are rejected. `i64::MAX` is accepted and converts exactly to `u64`. Path tests
   prove explicit override precedence/failure without XDG/HOME reads, absolute XDG selection,
   unset/empty/relative XDG
   fallback to absolute HOME, no file-existence fallback after valid XDG selection, failure without
   a usable base, the two exact conventional paths, and complete indifference to hostile
   `$MFM_DEPLOYMENT`. Fixtures prove no locator or resolved bootstrap-path value appears in TOML,
   redacted errors, or views.
9. **Endpoint authority/redaction** — proxy environment ignored; redirects not followed; nonlocal
   plaintext rejected; retry/referer disabled; exact production mode/root-store selection is tested.
   PostgreSQL and EVM real hermetic TLS handshakes both use the production checked PEM-root variant
   and reject a wrong pin, alternate unpinned CA, and wrong hostname; exact WebPKI mode selection is
   separately asserted. Hostile `PG*`/home/passfile authority is ignored without access or value
   logging; resolved private-locator contents and database/RPC credentials remain absent from
   startup stderr, request errors, stdout, HTTP body/headers, and debug output.
10. **CLI e2e** — exercise both the XDG default and explicit deployment override; provision and
    import; prove `entry-point list` reports the imported tag and `run start` rejects
    `--entry-point`; start `Current` without a RunId and capture the generated id/digest; re-start
    `Exact` with an explicit id, exercise exact mismatch after rebind, progress/show, delete config,
    show the retained run, paginate configs/runs, and check text/JSON exit behavior against managed
    reth and PostgreSQL.
11. **REST boundary without services** — prove the `serve` grammar passes absent/present
   `--deployment` values to the shared default/override loader before bind; exercise exact routed
   400/403/404/405/409/413/415/422/428/429/500/503/504 envelopes; conditional digest headers;
   both tagged start selections and rejection of nullable/untagged/entry-point forms; caller-owned
   path RunId with no server generation; `StartRunResult` and both indeterminate recovery envelopes;
   bounded extractor plus constructor re-check; strict query grammar; Host/Origin/media policy;
   raw terminal value bytes;
   semantically empty progress objects including whitespace; GET-equivalent HEAD admission,
   headers, and empty bodies; removable per-run guard and global semaphore. Socket tests cover
   owner/mode enforcement, active and stale leaves, mismatched markers/inodes, explicit recovery,
   graceful cleanup, and refusal to unlink foreign paths. Pre-router HTTP parser errors are not
   asserted to use the JSON envelope.
12. **Cross-transport managed parity** — build both binaries explicitly, install one strict TOML
    fixture under an isolated XDG default, pass the REST binary path to the harness/Nixfied task,
    and compare CLI JSON with REST for entry points, bindings, config summary/document/digest,
    `Current`/`Exact` start results under explicit RunIds, indeterminate recovery shapes, run heads,
    and full `RunView` including raw terminal bytes. CLI-only generated-id tests remain a named
    asymmetry. Do not rely on `CARGO_BIN_EXE_mfm_rest_api` from another package.
13. **Custody dependency boundary** — REST and shared request types contain no keystore-admin or
    secret parser dependency. Existing keystore `!Send/!Sync` compile-fail tests remain correctly
    described; no duplicate listener test claims more than they prove.

For a startup failure before bind, inspect process stderr only. Request-path leak tests first start a
valid daemon and then exercise a provider/store failure; they cannot expect an HTTP response from a
process that never bound.

---

# Logical implementation commits

Each commit leaves the workspace compiling with one current design and includes its owning docs.
Direct Cargo commands always run in the default Nix development shell.

1. **`add config catalog and index contracts`** — add `mfm-catalog`, checked names/digests/cursors,
   opaque custody records, catalog/index traits and outcomes, MemoryCatalog, and MemoryStore
   RunIndex/BTreeMap. Catalog reads return one owned name/digest/canonical snapshot. A test-only
   exhaustive wire fixture built from maximum accepted current domain values proves the shared 256
   KiB custody bound before any SQL constraint uses it; no dormant ConfigDocument or parallel
   Application API lands yet. Add workspace placement and the owning architecture/domain/catalog
   docs.
   Verify:
   `nix develop -c cargo test -p mfm-catalog -p mfm-store -p mfm-portfolio --all-targets`.
2. **`implement postgres client persistence authority`** — add the independent catalog baseline
   and gate, atomic quota/import/conditional-delete/indeterminate handling, and run-head paging.
   Reset the run baseline/gate to the final fixed-role ACL contract, add the split admin/runtime
   provisioner, checked raw-URI parser, ambient-input exclusion, target equivalence, TLS profile,
   shared checked TLS-root primitive, narrow SQLx patch, and exact runtime connection gate inside
   `mfm-storage-postgres`. Adapt every current `init`, `snapshot`, and `show` PostgreSQL caller plus
   the current combined CLI config/e2e to the split private locator environment inputs, and
   reject old owner-connected installations. Update design, storage/persisted-surface, CLI, Nixfied,
   and build-and-verification contracts in this commit. Add schema/role/connection-authority tests.
   The managed PostgreSQL task tests a real TLS handshake through the production content-pinned
   PEM-root private-locator variant, including
   wrong-pin, an alternate unpinned CA, and wrong-host rejection. Patch-level tests prove that the
   exclusive store contains exactly the parsed PEM anchors and no compiled WebPKI roots.
   Verify `nix run .#model-check`, focused crates, `nix run .#run -- --task postgres-test`, and
   `nix run .#run -- --task cli-e2e` while iterating.
3. **`replace one-route application composition`** — replace the singular assembly with the sole
   multi-route `ComposedRuntime` constructor and adapt the current CLI to pass a one-element checked
   binding set. The singular API is deleted here; CLI grammar and current Application execution
   entry points remain unchanged. Update domain/app/CLI docs for that current intermediate design.
   Verify:
   `nix develop -c cargo test -p mfm-app -p mfm-portfolio -p mfm-evm -p mfm --all-targets`.
4. **`cut over application and cli to stored configs`** — add the strict `deployment.toml` parser,
   XDG/HOME/override loader, checked live composer, and injected construction; consume the shared
   TLS-root primitive to harden exact EVM RPC
   authority in its adapter, add the async opaque `ConfigDocument` constructor, pure planner,
   digest/Program/C0 fixtures, `ConfigSelection`, exact planner-error mapping, `StartRunResult`,
   indeterminate recovery sums, and the complete typed Application surface. Add the pure RunId
   derivation helper/vectors and CLI-only OS entropy adapter; enable `serde_json/raw_value` for the
   shared exact raw-value serializer; rebuild CLI and its e2e across generated/explicit RunIds,
   `Current`/`Exact`, and default/overridden bootstrap paths; then atomically delete every
   superseded Application and CLI grammar/config/composition path. Update
   design/architecture/known-gap and CLI/live/storage/build-and-verification docs here, not later.
   Add a managed EVM `transport-authority-test` task with real TLS servers through the production
   content-pinned PEM-root path. Verify `nix run .#model-check`,
   `nix develop -c cargo test -p mfm-app -p mfm-evm-live -p mfm --all-targets`,
   `nix run .#run -- --task postgres-test`,
   `nix run .#run -- --task transport-authority-test`, and
   `nix run .#run -- --task cli-e2e` while iterating.
5. **`serve the application contract over rest`** — add Axum, REST routes, sum rendering,
   tagged `ConfigSelection` start bodies, caller-supplied path RunIds, `StartRunResult` and recovery
   rendering, conditional delete-digest headers, normalized routed errors,
   socket/media/bounds/backpressure/deadline controls, default/overridden bootstrap-path tests, REST
   README, and hermetic boundary tests; consume the shared serializers from commit 4 without adding
   a second representation.
   Verify: `nix develop -c cargo test -p mfm-rest-api --all-targets`.
6. **`prove cli and rest parity`** — add the explicit two-binary managed harness, Nixfied
   `rest-e2e`/CI composition, cross-transport start-result/recovery assertions, and
   build-and-verification documentation.
   Iterate with the one focused managed task; on the exact final candidate run
   `nix run .#model-check`, `nix flake check --no-build`, then one `nix run .#ci` and no redundant
   broad gates immediately before it.

The keystore RFC and its implementation form a separate high-risk sequence. They are not appended
as a partial seventh commit.

---

# Refused alternatives

| Refused | Reason |
| --- | --- |
| Inline config documents on run start | Duplicates bounds/parsing and bypasses the durable catalog decision. |
| `--entry-point` or an entry-point field on run start | The retained tagged document already selects it; a second selector can disagree. |
| Nullable digest in the shared start request | Hides two different assertions in correlated optional state; `ConfigSelection::Current/Exact` is exhaustive. |
| Unconditional config delete | A stale client can delete a newer revision. |
| Generic config merging/overrides | Creates per-entry-point partial schemas and ambiguous hashing. |
| Runs nested under configs | Retained runs survive config deletion and therefore are not config children. |
| Application-, Runtime-, or REST-server-derived RunId | Shared execution always receives explicit caller identity; only the CLI client boundary offers generation convenience. |
| Time/PID/counter RunId generation or random bytes used directly as a digest | Weak/ambient uniqueness or false `sha256-jcs-v1` provenance; the frozen random-preimage derivation is interoperable. |
| Run listing as generated-RunId recovery | Pagination and concurrency cannot prove which id belongs to an ambiguous attempt; the error carries exact recovery identity. |
| Status/entry point/timestamp in mechanical run listing | Requires semantic folds or a separately designed derived projection. |
| Planner registry / erased C0 | Adds runtime extensibility while defeating Runtime's typed start contract. |
| Deployment CRUD, identity, or catalog | Bootstrap authority is operator input, not a product resource or lifecycle. |
| Deployment/env inspection over REST | Exposes ambient-authority metadata clients do not need. |
| `$MFM_DEPLOYMENT`, directory search, includes, or interpolation | Creates hidden bootstrap precedence and additional ambient-resolution paths. |
| Locator values in `deployment.toml` | Risks persisting credentials/private authority and turns the bootstrap map into adapter configuration. |
| Integer-or-string deployment chain ids | Creates two representations; positive TOML `i64` is sufficient until a concrete upper-half `u64` requirement exists. |
| One PostgreSQL owner credential for provisioning and the daemon | Leaves the network process with unnecessary DDL authority. |
| Plaintext loopback PostgreSQL or EVM | A local listener can impersonate the endpoint, steal credentials, or forge evidence; every TCP backend is certificate verified. |
| Keystore import/list/delete over unauthenticated REST | Secret custody and metadata are outside the network surface. |
| Keystore placeholder CLI commands | No durable format exists; placeholders would be no-op or unsafe. |
| Passphrase-free authenticated key list/delete | Cryptographically impossible without a second integrity key. |
| Replay/trace/audit/export/effect placeholders | Those contracts were deleted or remain explicitly deferred. |
| Snapshot claims for keyset pages | Cross-request pages observe a changing data set. |
| One final documentation cleanup commit | Contracts and responsibility docs must land with their owning changes. |

---

# Material uncertainties

1. **Complete execution config versus reusable domain config.**
   *Assumption:* users normally reuse selector, routes, and Portfolio configuration together.
   *Why uncertain:* selecting several quotes against one Portfolio is a plausible daily workflow.
   *Consequence if wrong:* near-duplicate configs proliferate and users request unsafe generic
   overrides. *Validate:* confirm the intended operator workflow before commit 1; if separation is
   required, define it in a versioned domain entry-point input rather than catalog-level merging.

2. **Head-only run listing.**
   *Assumption:* RunId, head sequence/digest, total bytes, and individual `run show` are enough for
   the first discovery surface; generated-id and indeterminate recovery use the exact start output,
   never listing. *Why uncertain:* failed/recent/by-entry-point queries are common.
   *Consequence if wrong:* a separate non-authoritative run projection is needed. *Validate:* obtain
   the concrete listing questions before commit 1; never move status folding or recovery inference
   into Store.

3. **Owner-only REST socket approval.**
   *Assumption:* the user will accept replacing the earlier documented-only TCP bind with the
   required `0600` Unix socket. *Why uncertain:* the earlier decision explicitly preferred advice
   over enforcement. *Consequence if wrong:* unauthenticated REST cannot ship under a single-user
   security claim; arbitrary/loopback TCP trusts more principals than stated. *Validate:* obtain an
   explicit ruling before commit 5; authenticated TCP would require a separate authority design.

4. **Database and RPC backend profile.**
   *Assumption:* single-host TLS with either WebPKI or one content-pinned PEM CA bundle covers
   intended EVM and PostgreSQL deployments. *Why uncertain:* client certificates, cloud aliases,
   operating-system trust discovery, or PostgreSQL service files may be required operationally.
   *Consequence if wrong:* the deliberately narrow checked profile rejects a legitimate backend;
   it never falls back to plaintext. *Validate:* exercise intended PostgreSQL endpoint classes
   before commit 2 and EVM endpoint classes before commit 4, retain the pinned Rustls verification
   audit, and add a separate checked private-locator variant if required without widening
   `deployment.toml`.

5. **Admin/runtime database target equivalence.**
   *Assumption:* the supported PostgreSQL locator grammar can derive one unambiguous secret-free
   server/database identity while ignoring credentials and security-only parameters.
   *Why uncertain:* percent encoding, default ports, DNS aliases, and cloud connection aliases may
   name one target through distinct text. *Consequence if wrong:* the safe equality check rejects a
   legitimate split-credential deployment; it must never guess equivalence and mutate a different
   database. *Validate:* freeze normalization inside the already narrow single-host profile before
   commit 2, reject every ambiguous form, and test mismatch before any DDL.

6. **256 KiB config-document bound.**
   *Assumption:* it covers the worst canonical document admitted by the current 64-source domain
   while keeping per-request memory small. *Why uncertain:* maximum-length escaped public strings
   were not generated during this design review. *Consequence if wrong:* a domain-valid config
   cannot be imported. *Validate:* run verification item 2 before commit 1 freezes the constant;
   change the design bound, catalog constraint, and payload ceiling together if needed.

7. **REST deadline and memory envelope.**
   *Assumption:* a bounded deadline and small semaphore give acceptable behavior for 64-source runs
   and up-to-512 MiB retained reads. *Why uncertain:* provider deadlines compose sequentially and a
   valid cold read can be very large. *Consequence if wrong:* legitimate requests routinely time
   out or one request causes memory pressure. *Validate:* benchmark maximum current start/progress
   and large cold reads before selecting CLI flags/defaults in commit 5.

8. **Process topology.**
   *Assumption:* process-local REST guards are only amplification controls; correctness does not
   depend on one daemon or absence of direct CLI use. *Why uncertain:* operators may mistake them
   for global serialization. *Consequence if wrong:* duplicate provider IO and bypassed REST
   admission policy surprise operators. *Validate:* document and test two-process convergence; add
   no durable lease without a separate scheduler design.

9. **Keystore deferral and threat model.**
   *Assumption:* the requested key import/list/delete surface may wait for a dedicated custody RFC,
   and authenticated list/delete may require passphrase entry. *Why uncertain:* those actions
   were explicitly named as long-term goals, but key algorithms, recovery, platform/filesystem
   scope, and attacker model are unspecified. *Consequence if wrong:* this client implementation is
   blocked on custody rather than safely separable. *Validate:* obtain an explicit user ruling and
   external cryptographic review before any key grammar or persisted format is frozen.

10. **Pinned TLS integration seam.**
    *Assumption:* the selected reqwest/Rustls versions and the narrow SQLx patch can consume the same
    exact root-store representation without another implicit trust source. *Why uncertain:*
    reqwest's preconfigured-TLS hook is version-coupled and the SQLx ambient-free/exclusive-root
    constructor is a maintained workspace patch. *Consequence if wrong:* commits 2 and 4 cannot
    satisfy exact endpoint authority through the selected clients; silently using ambient or
    additive roots is forbidden. *Validate:* before commit 2, compile a minimal connector spike for
    both clients, inspect the resolved dependency graph/source, and run hostile-ambient plus
    alternate-CA handshakes against the exact lockfile.

11. **Deployment chain-id range.**
    *Assumption:* no intended EVM deployment needs a chain id above `i64::MAX`.
    *Why uncertain:* the domain intentionally stores chain ids as `u64`, while TOML integers are
    signed and current production fixtures exercise only small positive values.
    *Consequence if wrong:* `deployment.toml` cannot bind the upper half of the domain range, so
    such a config can never run through the production composer. *Validate:* inventory intended
    chain ids before commit 4; if an upper-half value is real, replace the TOML integer with one
    checked unsigned decimal-string representation and update fixtures/tests atomically.
